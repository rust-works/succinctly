//! #6616: `-R` bypasses jq's JSON parser, so `--stream` and `--stream-errors` change nothing under
//! it, as `--seq` does not (#6598): the lines are read and the files that did not open are reported
//! exactly as `-R` alone does. Succinctly kept the streaming readers' own placement of a line that
//! runs to the end of its file and reported such a failure up front. Every row was captured from
//! jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

const GONE: &str = "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n";
const GONE_BOTH: &str = "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/gone2.json: No such file or directory\n";

#[test]
fn raw_input_ignores_the_streaming_flags_for_unopened_files_6616() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    for (name, text) in [
        ("pnum.json", "1\n2"),
        ("empty.json", ""),
        ("nl.json", "a\nb\n"),
        ("fa.json", "[1,"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    for (flags, names, stdout, stderr, status) in [
        (
            &["-n", "--stream", "-c", "input"][..],
            &["pnum", "gone"][..],
            "\"1\"\n",
            "",
            0,
        ),
        (
            &["-n", "--stream", "-c", "[input,input]"][..],
            &["pnum", "gone"][..],
            "[\"1\",\"2\"]\n",
            GONE,
            2,
        ),
        (
            &["-n", "--stream-errors", "-c", "input"][..],
            &["pnum", "gone", "empty", "gone2"][..],
            "\"1\"\n",
            "",
            0,
        ),
        (
            &["--stream", "-c", "."][..],
            &["pnum", "gone", "empty", "gone2"][..],
            "\"1\"\n\"2\"\n",
            GONE_BOTH,
            2,
        ),
        (
            &["--stream-errors", "-c", "."][..],
            &["nl", "gone", "nl"][..],
            "\"a\"\n\"b\"\n\"a\"\n",
            GONE,
            2,
        ),
        (
            &["--stream", "-c", "."][..],
            &["fa", "gone", "nl"][..],
            "\"[1,a\"\n",
            GONE,
            2,
        ),
        (
            &["-s", "--stream", "-c", "."][..],
            &["nl", "gone"][..],
            "\"a\\nb\\n\"\n",
            GONE,
            2,
        ),
    ] {
        let files: Vec<String> = names
            .iter()
            .map(|name| format!("{d}/{name}.json"))
            .collect();
        let mut args = vec!["-R"];
        args.extend_from_slice(flags);
        args.extend(files.iter().map(String::as_str));
        let (out, err, code) = run_jq_full(&args, None)?;
        let (want_out, want_err) = (stdout.replace("@DIR@", &d), stderr.replace("@DIR@", &d));
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (want_out.as_str(), want_err.as_str(), status),
            "-R {flags:?} {names:?}"
        );
    }
    Ok(())
}
