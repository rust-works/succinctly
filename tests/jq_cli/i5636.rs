//! #5636: under `--seq`, text with no leading RS is never synced onto, so jq's first read of it
//! is `Unfinished abandoned text at EOF` -- an error `input` raises and `inputs` re-raises, exit
//! 5 -- where `succinctly jq` read no inputs and exited 0. Without `-n` it is still the warning.
//! Every row was captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

/// `(stdin, filter, expected stdout, expected stderr, expected exit status)`, run as
/// `-cn --seq <filter>`.
#[allow(clippy::type_complexity)]
const ROWS: &[(&str, &str, &str, &str, i32)] = &[
    (
        "1\n",
        "[inputs]",
        "",
        "jq: error (at <stdin>:1): Unfinished abandoned text at EOF at line 2, column 0\n",
        5,
    ),
    (
        "1",
        "[inputs]",
        "",
        "jq: error (at <stdin>:0): Unfinished abandoned text at EOF at line 1, column 1\n",
        5,
    ),
    (
        "1\n2\n",
        "[inputs]",
        "",
        "jq: error (at <stdin>:2): Unfinished abandoned text at EOF at line 3, column 0\n",
        5,
    ),
    (
        "",
        "[inputs]",
        "",
        "jq: error (at <stdin>:0): Unfinished abandoned text at EOF at line 1, column 0\n",
        5,
    ),
    (
        "1\n",
        "input",
        "",
        "jq: error (at <stdin>:1): Unfinished abandoned text at EOF at line 2, column 0\n",
        5,
    ),
    (
        "1\n",
        "first(inputs)",
        "",
        "jq: error (at <stdin>:1): Unfinished abandoned text at EOF at line 2, column 0\n",
        5,
    ),
    (
        "1\n",
        "reduce inputs as $x (0; .+1)",
        "",
        "jq: error (at <stdin>:1): Unfinished abandoned text at EOF at line 2, column 0\n",
        5,
    ),
    ("1\n", "try input catch \"caught\"", "\"caught\"\n", "", 0),
    (
        "1\n",
        "try ([inputs]) catch .",
        "\"Unfinished abandoned text at EOF at line 2, column 0\"\n",
        "",
        0,
    ),
    // A program that never reads is unaffected, and an RS-led record is read as before.
    ("1\n", "input_line_number", "0\n", "", 0),
    ("1\n", ".", "null\n", "", 0),
    ("\u{1e}1\n", "[inputs]", "[1]\n", "", 0),
];

#[test]
fn test_n_seq_input_raises_unfinished_abandoned_text_5636() -> Result<()> {
    for (stdin, filter, stdout, stderr, status) in ROWS {
        let (out, err, code) = run_jq_full(&["-cn", "--seq", filter], Some(stdin))?;
        let out = out.replace('\u{1e}', "");
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (*stdout, *stderr, *status),
            "{stdin:?} {filter}"
        );
    }
    Ok(())
}

/// The marker names the last source and the newlines in it; the message names the stream's end.
#[test]
fn test_the_marker_names_the_last_file_5636() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let write = |name: &str, bytes: &[u8]| -> Result<String> {
        let path = dir.path().join(name);
        std::fs::write(&path, bytes)?;
        Ok(path.to_str().unwrap().to_string())
    };
    let a = write("a.json", b"1\n2\n")?;
    let b = write("b.json", b"3")?;
    let empty = write("empty.json", b"")?;
    let rs = write("rs.json", b"\x1e4\n")?;
    for (files, filter, stdout, tail, status) in [
        (
            vec![&a, &b],
            "[inputs]",
            "",
            "b.json:0): Unfinished abandoned text at EOF at line 3, column 1\n",
            5,
        ),
        (
            vec![&a],
            "input",
            "",
            "a.json:2): Unfinished abandoned text at EOF at line 3, column 0\n",
            5,
        ),
        (
            vec![&a, &empty],
            "[inputs]",
            "",
            "empty.json:0): Unfinished abandoned text at EOF at line 3, column 0\n",
            5,
        ),
        (vec![&rs, &a], "[inputs]", "[4,1,2]\n", "", 0),
        (vec![&a, &rs], "[inputs]", "[4]\n", "", 0),
    ] {
        let mut args = vec!["-cn", "--seq", filter];
        args.extend(files.iter().map(|f| f.as_str()));
        let (out, err, code) = run_jq_full(&args, None)?;
        let out = out.replace('\u{1e}', "");
        assert!(err.ends_with(tail), "{filter} {files:?}: {err:?}");
        assert_eq!((out.as_str(), code), (stdout, status), "{filter} {files:?}");
    }
    Ok(())
}

/// Not under `-n`: the stream is a warning and the program runs on what is left (nothing).
#[test]
fn test_without_n_the_same_stream_still_only_warns_5636() -> Result<()> {
    let (out, err, code) = run_jq_full(&["-c", "--seq", "."], Some("1\n"))?;
    assert_eq!(
        (out.as_str(), err.as_str(), code),
        (
            "",
            "jq: ignoring parse error: Unfinished abandoned text at EOF at line 2, column 0\n",
            0
        )
    );
    Ok(())
}
