//! #6602: `-R -n` with a program that stops reading on a line before the one that runs to the end of
//! its file never reaches a file that did not open after it, so jq reports nothing and exits 0. A
//! program that reads the unterminated line (`[input,input]`) or every line reaches it and reports.
//! Succinctly reported every failure up front whenever a file before one lacked its final newline.
//! Every row was captured from jq 1.7.1; `--seq -R` takes `-R`'s route (#6598).

use anyhow::Result;

use crate::run_jq_full;

const GONE: &str = "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n";
const GONE_BOTH: &str = "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/gone2.json: No such file or directory\n";

#[test]
fn raw_input_reports_an_unopened_file_only_when_a_read_reaches_it_6602() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    for (name, text) in [
        ("pnum.json", "1\n2"),
        ("pq.json", "p\nq"),
        ("empty.json", ""),
        ("nl.json", "a\nb\n"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    for (flags, names, stdout, stderr, status) in [
        (
            &["-R", "-n", "-c", "input"][..],
            &["pnum", "gone", "empty", "gone2"][..],
            "\"1\"\n",
            "",
            0,
        ),
        (
            &["-R", "-n", "-c", "input"][..],
            &["pnum", "gone", "nl"][..],
            "\"1\"\n",
            "",
            0,
        ),
        (
            &["-R", "-n", "-c", "input"][..],
            &["pq", "gone", "nl"][..],
            "\"p\"\n",
            "",
            0,
        ),
        (
            &["-R", "-n", "-c", "first(inputs)"][..],
            &["pnum", "gone", "empty", "gone2"][..],
            "\"1\"\n",
            "",
            0,
        ),
        (
            &["-R", "-n", "-c", "try input catch \"E\""][..],
            &["pnum", "empty", "gone", "nl"][..],
            "\"1\"\n",
            "",
            0,
        ),
        (
            &["--seq", "-R", "-n", "-c", "input"][..],
            &["pq", "gone", "nl"][..],
            "\u{1e}\"p\"\n",
            "",
            0,
        ),
        // A read of the unterminated line goes on into the next file, and so reaches the failure.
        (
            &["-R", "-n", "-c", "[input,input]"][..],
            &["pnum", "gone", "nl"][..],
            "[\"1\",\"2a\"]\n",
            GONE,
            2,
        ),
        (
            &["-R", "-n", "-c", "[inputs]"][..],
            &["pq", "gone", "nl"][..],
            "[\"p\",\"qa\",\"b\"]\n",
            GONE,
            2,
        ),
        (
            &["-R", "-n", "-c", "[input,input,input]"][..],
            &["pq", "gone", "nl"][..],
            "[\"p\",\"qa\",\"b\"]\n",
            GONE,
            2,
        ),
        (
            &["-R", "-c", "."][..],
            &["pnum", "gone", "empty", "gone2"][..],
            "\"1\"\n\"2\"\n",
            GONE_BOTH,
            2,
        ),
        (
            &["-R", "-c", "input"][..],
            &["pq", "gone", "nl"][..],
            "\"qa\"\n",
            GONE,
            2,
        ),
    ] {
        let files: Vec<String> = names
            .iter()
            .map(|name| format!("{d}/{name}.json"))
            .collect();
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
