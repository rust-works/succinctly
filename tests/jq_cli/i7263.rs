//! #7263: `--seq` takes the bytes before the stream's first RS a run at a time, where it took them one
//! at a time (the reader's walk, `seq_no_rs_byte_position`, and the `fgets` chunks both of them count).
//! jq's reader discards those bytes, so all they leave behind is the line and column of every
//! diagnostic after them, and the run must keep both exactly. Each row pins what `/usr/bin/jq` 1.7.1
//! printed for the same files and argv, captured by hand: text and newlines before the first RS,
//! across two files, behind a byte-order mark, a run longer than an `fgets` chunk, and a stream
//! with no RS at all, with and without `-s` and `-n`'s `input`.

use std::path::Path;

use anyhow::Result;

use crate::spawn_jq;

const FILES: [(&str, &[u8]); 6] = [
    ("c1", b"ab\ncd\x1e{\"a\":1\n"),
    ("c2", b"ab\ncd\x1e{\"a\":1}\n\x1e[1,\n"),
    ("m1", b"x\ny"),
    ("m2", b"z\n\x1e1 2\n\x1e{\n"),
    ("b1", b"\xef\xbb\xbfab\ncd"),
    ("b2", b"more\nlines\n"),
];

/// `long` and `longrs`: 10,000 `x`, a newline and 5,000 `y`, then for `longrs` two records.
fn long_files(dir: &Path) -> Result<()> {
    let run = [vec![b'x'; 10_000], b"\n".to_vec(), vec![b'y'; 5_000]].concat();
    std::fs::write(dir.join("long"), &run)?;
    std::fs::write(
        dir.join("longrs"),
        [run.as_slice(), b"\x1e{\"a\":1}\n\x1e[1,"].concat(),
    )?;
    Ok(())
}

/// The argv before the files, the files, and jq's stdout, stderr and exit status.
type Row = (
    &'static [&'static str],
    &'static [&'static str],
    &'static str,
    &'static str,
    i32,
);

#[test]
fn text_before_the_first_rs_keeps_every_position_7263() -> Result<()> {
    let dir = tempfile::tempdir()?;
    for (name, bytes) in FILES {
        std::fs::write(dir.path().join(name), bytes)?;
    }
    long_files(dir.path())?;
    let rows: [Row; 13] = [
        (
            &["--seq", "-c", ".users[]|.id"],
            &["c1"],
            "",
            "jq: ignoring parse error: Unfinished JSON term at EOF at line 3, column 0\n",
            0,
        ),
        (
            &["--seq", "-c", "."],
            &["c2"],
            "\u{1e}{\"a\":1}\n",
            "jq: ignoring parse error: Unfinished JSON term at EOF at line 4, column 0\n",
            0,
        ),
        (
            &["--seq", "-c", "."],
            &["m1", "m2"],
            "\u{1e}1\n\u{1e}2\n",
            "jq: ignoring parse error: Unfinished JSON term at EOF at line 5, column 0\n",
            0,
        ),
        (
            &["--seq", "-c", "."],
            &["b1", "b2"],
            "",
            "jq: ignoring parse error: Unfinished abandoned text at EOF at line 4, column 0\n",
            0,
        ),
        (
            &["--seq", "-s", "-c", "."],
            &["b1", "b2"],
            "\u{1e}[]\n",
            "jq: ignoring parse error: Unfinished abandoned text at EOF at line 4, column 0\n",
            0,
        ),
        (
            &["--seq", "-n", "-c", "[inputs]"],
            &["m1", "m2"],
            "",
            "jq: error (at m2:3): Unfinished JSON term at EOF at line 5, column 0\n",
            5,
        ),
        (
            &["--seq", "-n", "input"],
            &["b1", "b2"],
            "",
            "jq: error (at b2:2): Unfinished abandoned text at EOF at line 4, column 0\n",
            5,
        ),
        (
            &["--seq", "-c", "."],
            &["long"],
            "",
            "jq: ignoring parse error: Unfinished abandoned text at EOF at line 2, column 5000\n",
            0,
        ),
        (
            &["--seq", "-s", "-c", "."],
            &["long"],
            "\u{1e}[]\n",
            "jq: ignoring parse error: Unfinished abandoned text at EOF at line 2, column 5000\n",
            0,
        ),
        (
            &["--seq", "-n", "-c", "[inputs]"],
            &["long"],
            "",
            "jq: error (at long:1): Unfinished abandoned text at EOF at line 2, column 5000\n",
            5,
        ),
        // jq's end-of-stream drop (#2998): the truncated last record is in the final buffer.
        (
            &["--seq", "-c", "."],
            &["longrs"],
            "\u{1e}{\"a\":1}\n",
            "",
            0,
        ),
        (
            &["--seq", "-s", "-c", "."],
            &["longrs"],
            "\u{1e}[{\"a\":1}]\n",
            "jq: ignoring parse error: Unfinished JSON term at EOF at line 3, column 4\n",
            0,
        ),
        (
            &["--seq", "-n", "-c", "[inputs]"],
            &["longrs"],
            "\u{1e}[{\"a\":1}]\n",
            "",
            0,
        ),
    ];
    for (args, files, stdout, stderr, code) in rows {
        let paths: Vec<String> = files
            .iter()
            .map(|f| dir.path().join(f).to_string_lossy().into_owned())
            .collect();
        let argv: Vec<&str> = args
            .iter()
            .copied()
            .chain(paths.iter().map(String::as_str))
            .collect();
        let (output, rc) = spawn_jq(&argv, None)?;
        // jq names a file by the path it was given; the rows pin the bare names.
        let prefix = format!("{}/", dir.path().to_string_lossy());
        let got_stderr = String::from_utf8(output.stderr)?.replace(&prefix, "");
        assert_eq!(
            (
                String::from_utf8(output.stdout)?.as_str(),
                got_stderr.as_str(),
                rc
            ),
            (stdout, stderr, code),
            "{args:?} {files:?}"
        );
    }
    Ok(())
}
