//! #6531: a last file that did not open is a source of its own in jq's reader, at line 0, and the
//! readers that keep their own positions say so: the closing event of `--stream` and
//! `--stream-errors` and the name `input_filename` gives after the end, a `--seq` fault at the end
//! of the text, and `-R`'s line read from past a file that did not open, which comes after the
//! open error where the lines before it come before. Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_last_unopened_file_is_the_source_the_end_is_read_at_6531() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    for (name, text) in [
        ("arr.json", "[1]"),
        ("t1.json", "[1,"),
        ("ab.json", "a\nb"),
        ("lines.json", "x\ny\n"),
        ("ok.json", "[1,2]\n"),
        ("seq.json", "\u{1e}1\n\u{1e}2\n"),
        ("nonl.json", "x"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    for (flags, names, stdout, stderr, status) in [
        (&["--stream", "-n", "-c", "[inputs], input_filename"][..], &["arr", "gone"][..], "[[[0],1],[[0]]]\n\"@DIR@/gone.json\"\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--stream", "-c", "[., input_filename]"][..], &["arr", "gone"][..], "[[[0],1],\"@DIR@/arr.json\"]\n[[[0]],\"@DIR@/gone.json\"]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--stream-errors", "-n", "-c", "[inputs], input_filename"][..], &["arr", "gone"][..], "[[[0],1],[[0]]]\n\"@DIR@/gone.json\"\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--stream", "-n", "-c", "[inputs], input_filename"][..], &["ok", "arr", "gone"][..], "[[[0],1],[[1],2],[[1]],[[0],1],[[0]]]\n\"@DIR@/gone.json\"\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--stream", "-n", "-c", "input, input_filename"][..], &["arr", "gone"][..], "[[0],1]\n\"@DIR@/arr.json\"\n", "", 0),
        (&["--seq", "-n", "-c", "input"][..], &["t1", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0): Unfinished abandoned text at EOF at line 1, column 3\n", 2),
        (&["--seq", "-n", "-c", "[inputs]"][..], &["t1", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0): Unfinished abandoned text at EOF at line 1, column 3\n", 2),
        (&["--seq", "-n", "-c", "[inputs], input_filename"][..], &["seq", "gone"][..], "\u{1e}[1,2]\n\u{1e}\"@DIR@/gone.json\"\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-R", "-c", "error"][..], &["ab", "gone"][..], "", "jq: error (at @DIR@/ab.json:1): a\njq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0): b\n", 2),
        (&["-R", "-c", "error"][..], &["lines", "gone", "ab"][..], "", "jq: error (at @DIR@/lines.json:1): x\njq: error (at @DIR@/lines.json:2): y\njq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/ab.json:1): a\n", 2),
        (&["-R", "-c", "error"][..], &["lines", "gone", "ab", "gone2"][..], "", "jq: error (at @DIR@/lines.json:1): x\njq: error (at @DIR@/lines.json:2): y\njq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/ab.json:1): a\n", 2),
        (&["-R", "-c", "error"][..], &["gone", "ab"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/ab.json:1): a\n", 2),
        (&["-R", "-c", "[., input_filename]"][..], &["nonl", "gone", "lines", "gone2"][..], "[\"xx\",\"@DIR@/lines.json\"]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
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
