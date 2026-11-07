//! #6522: `-R`, `--seq` and `--stream-errors` read past a fault, but jq's main loop still ends the
//! run one value past the first file that did not open: a file after the one that value comes from
//! is never opened, so it is never reported. Every row was captured from jq 1.7.1; the files end
//! where a line, a record or an event does, so the first value is complete inside its file.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_main_loop_that_reads_lines_records_or_events_stops_past_the_first_failure_6522() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    for (name, text) in [
        ("lines.json", "x\ny\n"),
        ("nums.json", "1\n2\n"),
        ("ok.json", "[1,2]\n"),
        ("seq.json", "\u{1e}1\n\u{1e}2\n"),
        ("empty.json", ""),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    for (flags, names, stdout, stderr, status) in [
        (&["-R", "-c", "."][..], &["gone", "lines", "gone2"][..], "\"x\"\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-R", "-c", "."][..], &["lines", "gone", "nums", "gone2"][..], "\"x\"\n\"y\"\n\"1\"\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-R", "-c", "[., input_filename]"][..], &["gone", "empty", "lines", "gone2"][..], "[\"x\",\"@DIR@/lines.json\"]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-R", "-c", "."][..], &["gone", "gone2", "lines", "gone3"][..], "\"x\"\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/gone2.json: No such file or directory\n", 2),
        (&["-R", "-c", "[., input]"][..], &["gone", "lines", "gone2"][..], "[\"x\",\"y\"]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-R", "-c", "-n", "[inputs]"][..], &["lines", "gone", "nums", "gone2"][..], "[\"x\",\"y\",\"1\",\"2\"]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/gone2.json: No such file or directory\n", 2),
        (&["--seq", "-c", "."][..], &["gone", "seq", "gone2"][..], "\u{1e}1\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--seq", "-c", "."][..], &["seq", "gone", "seq", "gone2"][..], "\u{1e}1\n\u{1e}2\n\u{1e}1\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--seq", "-c", "[., input_filename]"][..], &["gone", "empty", "seq", "gone2"][..], "\u{1e}[1,\"@DIR@/seq.json\"]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--stream-errors", "-c", "."][..], &["gone", "ok", "gone2"][..], "[[0],1]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--stream-errors", "-c", "."][..], &["ok", "gone", "nums", "gone2"][..], "[[0],1]\n[[1],2]\n[[1]]\n[[],1]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--stream-errors", "-c", "[., input_filename]"][..], &["gone", "empty", "ok", "gone2"][..], "[[[0],1],\"@DIR@/ok.json\"]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--stream-errors", "-c", "."][..], &["gone", "gone2", "ok", "gone3"][..], "[[0],1]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/gone2.json: No such file or directory\n", 2),
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
