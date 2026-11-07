//! #6555: a `--seq` stream that jq's `--seq` reader reads without a fault, under a program that reads
//! `input`, reports a file that did not open when a read reaches it, as a JSON stream's run does
//! (#6475, #6525), and not every one up front. That holds for `--stream` and `--stream-errors` over
//! the record stream too. Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_clean_seq_stream_reports_an_unopened_file_when_a_read_reaches_it_6555() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    for (name, text) in [
        ("seq.json", "\u{1e}1\n\u{1e}2\n"),
        ("rec.json", "\u{1e}[3]\n"),
        ("empty.json", ""),
        ("bad.json", "\u{1e}[1,\n\u{1e}2\n"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    #[rustfmt::skip]
    let rows = [
        (&["--seq", "-c", "input"][..], &["gone", "seq", "gone2"][..], "\u{1e}2\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--seq", "-c", "[., input]"][..], &["gone", "seq", "gone2"][..], "\u{1e}[1,2]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--seq", "-c", "[., input]"][..], &["seq", "gone", "seq", "gone2"][..], "\u{1e}[1,2]\n\u{1e}[1,2]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--seq", "-c", "[., input]"][..], &["seq", "gone", "gone2"][..], "\u{1e}[1,2]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/gone2.json: No such file or directory\n", 2),
        (&["--seq", "-c", "first(inputs)"][..], &["gone", "seq", "rec", "gone2"][..], "\u{1e}2\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--seq", "-c", "[., input_filename]"][..], &["gone", "empty", "seq", "gone2"][..], "\u{1e}[1,\"@DIR@/seq.json\"]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--seq", "-c", "-n", "input"][..], &["gone", "seq", "gone2"][..], "\u{1e}1\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--seq", "-c", "-n", "input"][..], &["seq", "gone", "gone2"][..], "\u{1e}1\n", "", 0),
        (&["--seq", "-c", "-n", "[inputs]"][..], &["seq", "gone", "rec", "gone2"][..], "\u{1e}[1,2,[3]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/gone2.json: No such file or directory\n", 2),
        (&["--seq", "-c", "-n", "[limit(2; inputs)]"][..], &["seq", "gone", "rec", "gone2"][..], "\u{1e}[1,2]\n", "", 0),
        (&["--seq", "--stream", "-c", "input"][..], &["gone", "seq", "gone2"][..], "\u{1e}[[],2]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--seq", "--stream", "-c", "-n", "input"][..], &["seq", "gone", "gone2"][..], "\u{1e}[[],1]\n", "", 0),
        (&["--seq", "--stream-errors", "-c", "[., input]"][..], &["gone", "seq", "gone2"][..], "\u{1e}[[[],1],[[],2]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--seq", "--stream-errors", "-c", "-n", "input"][..], &["gone", "rec", "gone2"][..], "\u{1e}[[0],3]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        // A stream its reader faults on keeps its up-front report, which jq agrees with here.
        (&["--seq", "-c", "[., input]"][..], &["bad", "gone", "gone2"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/gone2.json: No such file or directory\njq: error (at @DIR@/gone2.json:0): break\n", 2),
    ];
    for (flags, names, stdout, stderr, status) in rows {
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
