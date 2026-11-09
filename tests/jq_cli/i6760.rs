//! #6760: jq reads a file with `fgets` into a 4,096-byte buffer. When a file's last read fills all
//! 4,095 bytes with no newline, EOF has not been seen, so jq reads that file once more and the read
//! returns nothing; a closing event the file held back comes out of that read, in that file. The
//! stream readers (`--seq --stream`, `--stream`, `--stream-errors`) made no such read, so the event
//! came out of the next file's read instead: at the wrong `input_filename`, past a directory that
//! should have stopped the loop first, and with a different exit status for `input`.
//!
//! The rows hold a file at 4,094, 4,095 and 4,096 bytes so the size test cannot pass by accident,
//! and a last read of 4,095 bytes after an earlier newline. Every expected value was captured from
//! jq 1.7.1 (`/usr/bin/jq`); `short`, `long` and `two` are rows that never moved. The `gone` and `dir`
//! rows after a full file are the ones `closing_event_lags` judged by the bytes alone, which put the
//! held-back closer at the failed file's read although the file's own extra read had delivered it.

use anyhow::Result;

use crate::run_jq_full;

/// One row: the flags and program, the files under the fixture directory, and jq 1.7.1's stdout,
/// stderr and exit status, with `@DIR@` for the directory.
type Row<'a> = (&'a [&'a str], &'a [&'a str], &'a str, &'a str, i32);

#[test]
fn a_last_read_that_fills_the_buffer_is_followed_by_an_empty_read_6760() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::create_dir(dir.path().join("dir"))?;
    // `\x1e[1`, spaces, `]`: a record whose closing event jq holds back until the next read.
    let seq = |spaces: usize| [&b"\x1e[1"[..], &vec![b' '; spaces], b"]"].concat();
    let late = [&b"\x1e7\n"[..], &seq(4091)].concat();
    let full_json = [&b"[1"[..], &vec![b' '; 4092], b"]"].concat();
    for (name, bytes) in [
        ("full.seq", seq(4091)),
        ("short.seq", seq(4090)),
        ("long.seq", seq(4092)),
        ("late.seq", late),
        ("full.json", full_json),
        ("arr", b"\x1e[1]".to_vec()),
        ("two", b"\x1e2\n".to_vec()),
        ("p.json", b"[7]".to_vec()),
    ] {
        std::fs::write(dir.path().join(name), bytes)?;
    }
    assert_eq!(std::fs::metadata(dir.path().join("full.seq"))?.len(), 4095);
    assert_eq!(std::fs::metadata(dir.path().join("short.seq"))?.len(), 4094);
    assert_eq!(std::fs::metadata(dir.path().join("long.seq"))?.len(), 4096);
    assert_eq!(std::fs::metadata(dir.path().join("full.json"))?.len(), 4095);
    #[rustfmt::skip]
    let rows: [Row<'_>; 15] = [
        (&["-c", "--seq", "--stream", "."][..], &["full.seq", "arr"][..], "\u{1e}[[0],1]\n\u{1e}[[0]]\n", "", 0),
        (&["-c", "--seq", "--stream-errors", "."][..], &["full.seq", "arr"][..], "\u{1e}[[0],1]\n\u{1e}[[0]]\n", "", 0),
        (&["-c", "--seq", "--stream", "[., input_filename]"][..], &["full.seq", "arr"][..], "\u{1e}[[[0],1],\"@DIR@/full.seq\"]\n\u{1e}[[[0]],\"@DIR@/full.seq\"]\n", "", 0),
        (&["-c", "--seq", "--stream", "[., input]"][..], &["full.seq", "arr"][..], "\u{1e}[[[0],1],[[0]]]\n", "", 0),
        (&["-c", "--stream", "[., input_filename]"][..], &["full.json", "p.json"][..], "[[[0],1],\"@DIR@/full.json\"]\n[[[0]],\"@DIR@/full.json\"]\n[[[0],7],\"@DIR@/p.json\"]\n[[[0]],null]\n", "", 0),
        (&["-c", "--seq", "--stream", "."][..], &["late.seq", "arr"][..], "\u{1e}[[],7]\n\u{1e}[[0],1]\n\u{1e}[[0]]\n", "", 0),
        (&["-c", "--seq", "--stream", "."][..], &["full.seq", "dir", "arr"][..], "\u{1e}[[0],1]\n\u{1e}[[0]]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "--seq", "--stream", "[., input_filename]"][..], &["full.seq", "dir", "arr"][..], "\u{1e}[[[0],1],\"@DIR@/full.seq\"]\n\u{1e}[[[0]],\"@DIR@/full.seq\"]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "--seq", "--stream", "."][..], &["short.seq", "arr"][..], "\u{1e}[[0],1]\n\u{1e}[[0]]\n\u{1e}[[0],1]\n\u{1e}[[0]]\n", "", 0),
        (&["-c", "--seq", "--stream", "."][..], &["long.seq", "arr"][..], "\u{1e}[[0],1]\n\u{1e}[[0]]\n\u{1e}[[0],1]\n\u{1e}[[0]]\n", "", 0),
        (&["-c", "--seq", "--stream", "."][..], &["full.seq", "two"][..], "\u{1e}[[0],1]\n\u{1e}[[0]]\n\u{1e}[[],2]\n", "", 0),
        (&["-c", "--stream-errors", "."][..], &["full.json", "gone", "gone2"][..], "[[0],1]\n[[0]]\n", "jq: error: Could not open file @DIR@/gone: No such file or directory\njq: error: Could not open file @DIR@/gone2: No such file or directory\n", 2),
        (&["-c", "--stream-errors", "."][..], &["full.json", "dir", "gone"][..], "[[0],1]\n[[0]]\n", "jq: error: Is a directory\njq: error: Could not open file @DIR@/gone: No such file or directory\n", 2),
        (&["-c", "--seq", "--stream", "."][..], &["full.seq", "gone"][..], "\u{1e}[[0],1]\n\u{1e}[[0]]\n", "jq: error: Could not open file @DIR@/gone: No such file or directory\n", 2),
        (&["-c", "--seq", "--stream", "."][..], &["short.seq", "gone"][..], "\u{1e}[[0],1]\n\u{1e}[[0]]\n", "jq: error: Could not open file @DIR@/gone: No such file or directory\n", 2),
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
