//! #6589: `--seq -s` beside a file that does not open. jq's reader opens the files in order and
//! reports a failure when it reaches that file, so a warning met in a file before it comes first.
//! Its main loop reads the whole slurp in one call, and stops at the first warning met once a file
//! has failed: the array is then lost, and nothing after that warning is read or reported. A
//! warning before every failure keeps the array, and so does a run whose only warning is the EOF
//! report jq defers past the array (which the main loop then never reads, though `input` does). A
//! last file that did not open is the empty buffer the stream ends in, so its EOF report is not
//! deferred. `-s` with a program that reads `input` tripped a debug assertion over a lost array.
//! Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_seq_slurp_reports_an_unopened_file_where_its_reader_reaches_it_6589() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    for (name, text) in [
        ("bad", "\u{1e}1\n\u{1e}{\"a\n\u{1e}2\n"),
        ("norsend", "\u{1e}7"),
        ("defer", "\u{1e}1{"),
        ("s2", "\u{1e}[1]\n\u{1e}[2]\n"),
    ] {
        std::fs::write(dir.path().join(format!("{name}.json")), text)?;
    }
    for (program, names, stdout, stderr, status) in [
        (".", &["bad", "gone"][..], "\u{1e}[1,2]\n", "jq: ignoring parse error: Truncated value at line 3, column 1\njq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (".", &["gone", "bad"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: ignoring parse error: Truncated value at line 3, column 1\n", 2),
        (".", &["bad", "norsend", "gone"][..], "", "jq: ignoring parse error: Truncated value at line 3, column 1\njq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: ignoring parse error: Potentially truncated top-level numeric value at EOF at line 4, column 2\n", 2),
        (".", &["gone", "bad", "gone2"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: ignoring parse error: Truncated value at line 3, column 1\n", 2),
        (".", &["bad", "gone", "s2"][..], "\u{1e}[1,2,[1],[2]]\n", "jq: ignoring parse error: Truncated value at line 3, column 1\njq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        ("input", &["bad", "gone"][..], "", "jq: ignoring parse error: Truncated value at line 3, column 1\njq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0): break\n", 2),
        ("[inputs]", &["bad", "gone"][..], "\u{1e}[]\n", "jq: ignoring parse error: Truncated value at line 3, column 1\njq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        ("input_filename", &["bad", "gone"][..], "\u{1e}\"@DIR@/gone.json\"\n", "jq: ignoring parse error: Truncated value at line 3, column 1\njq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        ("input", &["gone", "bad"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: ignoring parse error: Truncated value at line 3, column 1\n", 2),
        (".", &["defer", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: ignoring parse error: Unfinished JSON term at EOF at line 1, column 3\n", 2),
        (".", &["gone", "defer"][..], "\u{1e}[1]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        ("input", &["gone", "defer"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at <unknown>): Unfinished JSON term at EOF at line 1, column 3\n", 2),
        (".", &["defer"][..], "\u{1e}[1]\n", "jq: ignoring parse error: Unfinished JSON term at EOF at line 1, column 3\n", 0),
    ] {
        let files: Vec<String> = names.iter().map(|name| format!("{d}/{name}.json")).collect();
        let mut args = vec!["--seq", "-c", "-s", program];
        args.extend(files.iter().map(String::as_str));
        let (out, err, code) = run_jq_full(&args, None)?;
        let expected = (stdout.replace("@DIR@", &d), stderr.replace("@DIR@", &d));
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (expected.0.as_str(), expected.1.as_str(), status),
            "{program} {names:?}"
        );
    }
    Ok(())
}
