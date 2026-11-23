//! #7842: under `--stream`, a file that starts with a UTF-8 byte-order mark after a file that does
//! not open (one that is missing, or a directory) has its mark skipped, as jq skips the mark at the
//! front of its input: a source that does not open gives the stream no bytes. The stream reader
//! takes each source's share of the mark by the source's place in the list, and the sources that did
//! not open had no share, so the mark's cut fell on the wrong source and the mark was read as text
//! (`Invalid numeric literal at line 1, column 4`). A mark cut across files (#7566) goes on past a
//! source that does not open too.
//!
//! Every expectation was captured from jq 1.7.1 (`/usr/bin/jq`); jq 1.8.2 agrees on every row.

use anyhow::Result;

use crate::run_jq_in_dir;

/// `(flags, files, stdout, stderr, exit status)`; `-` is standard input, which holds a mark and
/// `[1]`.
type Row = (
    &'static [&'static str],
    &'static [&'static str],
    &'static str,
    &'static str,
    i32,
);

const GONE: &str = "jq: error: Could not open file gone.json: No such file or directory\n";
const DIR: &str = "jq: error: Is a directory\n";

static ROWS: &[Row] = &[
    (
        &["-c", "--stream", "."],
        &["gone", "bom"],
        "[[0],1]\n",
        GONE,
        2,
    ),
    (
        &["-c", "--stream", "."],
        &["adir", "bom"],
        "[[0],1]\n",
        DIR,
        2,
    ),
    (
        &["-c", "--stream", "."],
        &["gone", "-"],
        "[[0],1]\n",
        GONE,
        2,
    ),
    (
        &["-c", "--stream-errors", "."],
        &["gone", "bom"],
        "[[0],1]\n",
        GONE,
        2,
    ),
    (
        &["-c", "--stream-errors", "."],
        &["adir", "bom"],
        "[[0],1]\n",
        DIR,
        2,
    ),
    (
        &["-c", "-n", "--stream", "[inputs]"],
        &["gone", "bom"],
        "[[[0],1],[[0]]]\n",
        GONE,
        2,
    ),
    (
        &["-c", "-n", "--stream", "[inputs]"],
        &["adir", "bom"],
        "[[[0],1],[[0]]]\n",
        DIR,
        2,
    ),
    (
        &["-c", "-n", "--stream", "input"],
        &["gone", "bom"],
        "[[0],1]\n",
        GONE,
        2,
    ),
    (
        &["-c", "-e", "--stream", "."],
        &["gone", "bom"],
        "[[0],1]\n",
        GONE,
        2,
    ),
    (
        &["-c", "--stream", "."],
        &["gone", "gone2", "bom"],
        "[[0],1]\n",
        "jq: error: Could not open file gone.json: No such file or directory\n\
         jq: error: Could not open file gone2.json: No such file or directory\n",
        2,
    ),
    (
        &["-c", "--stream", "."],
        &["gone", "adir", "bom"],
        "[[0],1]\n",
        "jq: error: Could not open file gone.json: No such file or directory\n\
         jq: error: Is a directory\n",
        2,
    ),
    (
        &["-c", "--stream", "."],
        &["adir", "gone", "bom"],
        "[[0],1]\n",
        "jq: error: Is a directory\n\
         jq: error: Could not open file gone.json: No such file or directory\n",
        2,
    ),
    (
        &["-c", "--stream", "."],
        &["gone", "bom", "ok"],
        "[[0],1]\n",
        GONE,
        2,
    ),
    (
        &["-c", "--stream", "."],
        &["gone", "bom", "gone"],
        "[[0],1]\n",
        GONE,
        2,
    ),
    // The mark's first byte, a source that does not open, then the rest of the mark.
    (
        &["-c", "--stream", "."],
        &["ef", "gone", "bbbf"],
        "[[0],2]\n",
        GONE,
        2,
    ),
    (
        &["-c", "--stream", "."],
        &["ef", "adir", "bbbf"],
        "[[0],2]\n",
        DIR,
        2,
    ),
    (
        &["-c", "--stream-errors", "."],
        &["ef", "gone", "bbbf"],
        "[[0],2]\n",
        GONE,
        2,
    ),
    // A mark that is not at the front of the stream is text, as before.
    (
        &["-c", "--stream", "."],
        &["ok", "gone", "bom"],
        "[[0],3]\n[[0]]\n",
        "jq: error: Could not open file gone.json: No such file or directory\n\
         jq: parse error: Invalid numeric literal at line 2, column 4\n",
        2,
    ),
    (
        &["-c", "--stream", "."],
        &["bom", "gone", "bom"],
        "[[0],1]\n[[0]]\n",
        "jq: error: Could not open file gone.json: No such file or directory\n\
         jq: parse error: Invalid numeric literal at line 2, column 4\n",
        2,
    ),
];

#[test]
fn a_byte_order_mark_after_a_file_that_does_not_open_is_skipped_under_stream_7842() -> Result<()> {
    let dir = tempfile::tempdir()?;
    for (name, bytes) in [
        ("bom.json", &b"\xef\xbb\xbf[1]\n"[..]),
        ("ef.json", b"\xef"),
        ("bbbf.json", b"\xbb\xbf[2]\n"),
        ("ok.json", b"[3]\n"),
    ] {
        std::fs::write(dir.path().join(name), bytes)?;
    }
    std::fs::create_dir(dir.path().join("adir.json"))?;
    for &(flags, names, stdout, stderr, status) in ROWS {
        let files: Vec<String> = names
            .iter()
            .map(|name| match *name {
                "-" => "-".to_owned(),
                name => format!("{name}.json"),
            })
            .collect();
        let mut args: Vec<&str> = flags.to_vec();
        args.extend(files.iter().map(String::as_str));
        let got = run_jq_in_dir(dir.path(), &args, Some("\u{feff}[1]\n"))?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (stdout, stderr, status),
            "{flags:?} {names:?}"
        );
    }
    Ok(())
}
