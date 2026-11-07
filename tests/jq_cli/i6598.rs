//! #6598, #6587: `-R` takes over from `--seq` on input, so `--seq -R` reports the files that did not
//! open exactly as `-R` alone does, and `--seq` only frames the output with RS. A failure past an
//! empty file (#6598) and the one after a line that runs to the end of its file (#6587) were each
//! dropped. Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

const GONE: &str = "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n";
const GONE_BOTH: &str = "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/gone2.json: No such file or directory\n";

#[test]
fn seq_raw_input_reports_unopened_files_as_raw_input_does_6598() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    for (name, text) in [
        ("pnum.json", "1\n2"),
        ("empty.json", ""),
        ("nl.json", "a\nb\n"),
        ("one.json", "x"),
        ("c.json", "\u{1e}4"),
        ("b.json", "\u{1e}[3]\n"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    for (flags, names, stdout, stderr, status) in [
        (
            &["-c", "."][..],
            &["pnum", "gone", "empty", "gone2"][..],
            "\u{1e}\"1\"\n\u{1e}\"2\"\n",
            GONE_BOTH,
            2,
        ),
        (
            &["-c", "."][..],
            &["nl", "gone", "one", "gone2"][..],
            "\u{1e}\"a\"\n\u{1e}\"b\"\n\u{1e}\"x\"\n",
            GONE_BOTH,
            2,
        ),
        (
            &["-c", "empty"][..],
            &["pnum", "gone", "empty", "gone2"][..],
            "",
            GONE_BOTH,
            2,
        ),
        // #6587: the line of `c` has no newline, so the reader asks `gone2` for the rest of it.
        (
            &["-c", "."][..],
            &["gone", "c", "gone2"][..],
            "\u{1e}\"\\u001e4\"\n",
            GONE_BOTH,
            2,
        ),
        (
            &["-c", "."][..],
            &["b", "gone", "c", "gone2"][..],
            "\u{1e}\"\\u001e[3]\"\n\u{1e}\"\\u001e4\"\n",
            GONE_BOTH,
            2,
        ),
        (
            &["-n", "-c", "input"][..],
            &["pnum", "gone"][..],
            "\u{1e}\"1\"\n",
            "",
            0,
        ),
        (
            &["-n", "-c", "input"][..],
            &["gone", "pnum", "gone2"][..],
            "\u{1e}\"1\"\n",
            GONE,
            2,
        ),
        (
            &["-n", "-c", "first(inputs)"][..],
            &["nl", "gone", "gone2"][..],
            "\u{1e}\"a\"\n",
            "",
            0,
        ),
        (
            &["-n", "-c", "[inputs]"][..],
            &["nl", "gone", "one", "gone2"][..],
            "\u{1e}[\"a\",\"b\",\"x\"]\n",
            GONE_BOTH,
            2,
        ),
    ] {
        let files: Vec<String> = names
            .iter()
            .map(|name| format!("{d}/{name}.json"))
            .collect();
        let (want_out, want_err) = (stdout.replace("@DIR@", &d), stderr.replace("@DIR@", &d));
        let mut args = vec!["--seq", "-R"];
        args.extend_from_slice(flags);
        args.extend(files.iter().map(String::as_str));
        let (out, err, code) = run_jq_full(&args, None)?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (want_out.as_str(), want_err.as_str(), status),
            "--seq -R {flags:?} {names:?}"
        );
        // `-R` alone reads the same and reports the same; only the RS framing goes.
        let (out, err, code) = run_jq_full(&args[1..], None)?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (
                want_out.replace('\u{1e}', "").as_str(),
                want_err.as_str(),
                status
            ),
            "-R {flags:?} {names:?}"
        );
    }
    Ok(())
}
