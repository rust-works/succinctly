//! #6555: `--seq` reports a file that did not open when its reader reaches it, as jq's does, and
//! not up front: a run that stops reading before a missing file says nothing of it. A last file
//! that did not open is one more source to jq's `--seq` readers, an empty one they stand at, line
//! 0, so a record the end-of-stream drop (#2998) would take is kept, a fault at the end of the text
//! is still raised, and both are placed at that file. Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn seq_reports_an_unopened_file_when_its_reader_reaches_it_6555() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    for (name, text) in [
        ("seq.json", "\u{1e}1\n\u{1e}2\n"),
        ("one.json", "\u{1e}5\n"),
        ("norsend.json", "\u{1e}7"),
        ("arr.json", "\u{1e}[1]"),
        ("empty.json", ""),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    for (flags, names, stdout, stderr, status) in [
        (&["--seq", "-c", "input"][..], &["gone", "seq", "gone2"][..], "\u{1e}2\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--seq", "-c", "[., input]"][..], &["seq", "gone", "gone2"][..], "\u{1e}[1,2]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/gone2.json: No such file or directory\n", 2),
        (&["--seq", "-c", "."][..], &["seq", "gone", "one", "gone2"][..], "\u{1e}1\n\u{1e}2\n\u{1e}5\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--seq", "-n", "-c", "input"][..], &["seq", "gone"][..], "\u{1e}1\n", "", 0),
        (&["--seq", "-n", "-c", "input"][..], &["one", "gone"][..], "\u{1e}5\n", "", 0),
        (&["--seq", "-n", "-c", "[inputs]"][..], &["seq", "gone", "one"][..], "\u{1e}[1,2,5]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--seq", "-c", "."][..], &["empty", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: ignoring parse error: Unfinished abandoned text at EOF at line 1, column 0\n", 2),
        (&["--seq", "-c", "input"][..], &["gone", "empty"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: ignoring parse error: Unfinished abandoned text at EOF at line 1, column 0\n", 2),
        (&["--seq", "-c", "."][..], &["seq", "arr", "gone"][..], "\u{1e}1\n\u{1e}2\n\u{1e}[1]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--seq", "-c", "[., input_filename, input_line_number]"][..], &["seq", "arr", "gone"][..], "\u{1e}[1,\"@DIR@/seq.json\",1]\n\u{1e}[2,\"@DIR@/seq.json\",2]\n\u{1e}[[1],\"@DIR@/arr.json\",0]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--seq", "-c", "."][..], &["seq", "norsend", "gone"][..], "\u{1e}1\n\u{1e}2\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: ignoring parse error: Potentially truncated top-level numeric value at EOF at line 3, column 2\n", 2),
        (&["--seq", "-c", "[inputs]"][..], &["seq", "norsend", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0): Potentially truncated top-level numeric value at EOF at line 3, column 2\n", 2),
        (&["--seq", "-n", "-c", "input"][..], &["seq", "norsend", "gone"][..], "\u{1e}1\n", "", 0),
        (&["--seq", "--stream", "-c", "input"][..], &["empty", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: ignoring parse error: Unfinished abandoned text at EOF at line 1, column 0\n", 2),
        (&["--seq", "--stream", "-c", "[., input_filename, input_line_number]"][..], &["seq", "norsend", "gone"][..], "\u{1e}[[[],1],\"@DIR@/seq.json\",1]\n\u{1e}[[[],2],\"@DIR@/seq.json\",2]\n\u{1e}[[[],7],\"@DIR@/gone.json\",0]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--seq", "--stream", "-c", "[., input_filename, input_line_number]"][..], &["arr", "gone"][..], "\u{1e}[[[0],1],\"@DIR@/arr.json\",0]\n\u{1e}[[[0]],\"@DIR@/gone.json\",0]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--seq", "--stream-errors", "-n", "-c", "[input, input_filename, input_line_number]"][..], &["empty", "gone"][..], "\u{1e}[[\"Unfinished abandoned text at EOF at line 1, column 0\",[]],\"@DIR@/gone.json\",0]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--seq", "--stream-errors", "-n", "-c", "input"][..], &["seq", "gone"][..], "\u{1e}[[],1]\n", "", 0),
    ] {
        let files: Vec<String> = names.iter().map(|name| format!("{d}/{name}.json")).collect();
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
