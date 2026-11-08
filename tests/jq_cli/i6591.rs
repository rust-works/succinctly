//! #6591: `--stream-errors`, next to a file that does not open.
//!
//! - jq's main loop stops one event past the first open failure, so a first event that is complete
//!   inside its file (`[1,` has `[[0],1]`, a fault after it notwithstanding) never opens the file
//!   after it; one that is not (`[1`, a bare `1`) waits for the next file, which is read.
//! - The event of a parse error at the end of the text is read at a last file that did not open,
//!   line 0 (`input_filename` is that file), as the JSON reader's fault is (#6480).
//!
//! - `--seq --stream` without a program that reads `input` warns about a fault as jq's main loop
//!   pops it, so a failure sits among the warnings by the file it is before, and a warning met
//!   after the value that took the reader past the failure is never read.
//!
//! Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn stream_errors_stops_past_the_first_failure_and_places_its_end_fault_6591() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    for (name, text) in [
        ("jbad.json", "[1,"),
        ("half.json", "[1"),
        ("ok.json", "[1,2]\n"),
        ("bare.json", "1"),
        ("line.json", "1\n"),
        ("empty.json", ""),
        ("str.json", "\"ab"),
        ("br.json", "]"),
        ("sp.json", " "),
        ("tail.json", ",2]"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    #[rustfmt::skip]
    let rows = [
        (&["--stream-errors", "-c", "."][..], &["gone", "jbad", "gone2"][..], "[[0],1]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--stream-errors", "-c", "."][..], &["gone", "jbad", "gone2", "gone3"][..], "[[0],1]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--stream-errors", "-c", "."][..], &["jbad", "gone", "gone2"][..], "[[0],1]\n[\"Unfinished JSON term at EOF at line 1, column 3\",[1]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/gone2.json: No such file or directory\n", 2),
        (&["--stream-errors", "-c", "."][..], &["gone", "empty", "jbad", "gone2"][..], "[[0],1]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--stream-errors", "-c", "."][..], &["gone", "half", "gone2"][..], "[\"Unfinished JSON term at EOF at line 1, column 2\",[0]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/gone2.json: No such file or directory\n", 2),
        (&["--stream-errors", "-c", "."][..], &["gone", "half", "line", "gone2"][..], "[\"Unfinished JSON term at EOF at line 2, column 0\",[0]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/gone2.json: No such file or directory\n", 2),
        (&["--stream-errors", "-c", "."][..], &["gone", "bare", "gone2"][..], "[[],1]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/gone2.json: No such file or directory\n", 2),
        (&["--stream-errors", "-c", "."][..], &["gone", "line", "gone2"][..], "[[],1]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--stream-errors", "-c", "."][..], &["gone", "ok", "gone2"][..], "[[0],1]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--stream-errors", "-c", "[., input]"][..], &["gone", "jbad", "gone2"][..], "[[[0],1],[\"Unfinished JSON term at EOF at line 1, column 3\",[1]]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/gone2.json: No such file or directory\n", 2),
        (&["--stream-errors", "-c", "[., input_filename, input_line_number]"][..], &["jbad", "gone"][..], "[[[0],1],\"@DIR@/jbad.json\",0]\n[[\"Unfinished JSON term at EOF at line 1, column 3\",[1]],\"@DIR@/gone.json\",0]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--stream-errors", "-c", "[., input_filename, input_line_number]"][..], &["half", "gone"][..], "[[\"Unfinished JSON term at EOF at line 1, column 2\",[0]],\"@DIR@/gone.json\",0]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--stream-errors", "-c", "[., input_filename, input_line_number]"][..], &["jbad", "gone", "gone2"][..], "[[[0],1],\"@DIR@/jbad.json\",0]\n[[\"Unfinished JSON term at EOF at line 1, column 3\",[1]],\"@DIR@/gone2.json\",0]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/gone2.json: No such file or directory\n", 2),
        // No missing file: the fault event is read in the file itself.
        (&["--stream-errors", "-c", "[., input_filename, input_line_number]"][..], &["jbad"][..], "[[[0],1],\"@DIR@/jbad.json\",0]\n[[\"Unfinished JSON term at EOF at line 1, column 3\",[1]],null,0]\n", "", 0),
        (&["--stream-errors", "-c", "-n", "[inputs, input_filename]"][..], &["jbad", "gone"][..], "[[[0],1],[\"Unfinished JSON term at EOF at line 1, column 3\",[1]],\"@DIR@/gone.json\"]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        // A file before the failure that ends inside a token hands it on to the next one, so the
        // first event past the failure is not complete in the next file alone.
        (&["--stream-errors", "-c", "."][..], &["str", "gone", "br", "gone2"][..], "[\"Unfinished string at EOF at line 1, column 4\",[]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/gone2.json: No such file or directory\n", 2),
        (&["--stream-errors", "-c", "."][..], &["str", "gone", "tail", "gone2"][..], "[\"Unfinished string at EOF at line 1, column 6\",[]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/gone2.json: No such file or directory\n", 2),
        (&["--stream-errors", "-c", "."][..], &["bare", "gone", "sp", "gone2"][..], "[[],1]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--stream-errors", "-c", "."][..], &["line", "gone", "sp", "gone2"][..], "[[],1]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/gone2.json: No such file or directory\n", 2),
    ];
    for (flags, names, stdout, stderr, status) in rows {
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

#[test]
fn seq_stream_places_a_warning_among_the_open_failures_6591() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    for (name, text) in [
        ("bad", "\u{1e}1\n\u{1e}{\"a\n\u{1e}2\n"),
        ("d", "\u{1e}[1,"),
        ("ok", "\u{1e}3\n\u{1e}4\n"),
        ("mid", "\u{1e}1\n\u{1e}{\"a\n\u{1e}2\n\u{1e}5\n"),
    ] {
        std::fs::write(dir.path().join(format!("{name}.seq")), text)?;
    }
    const WARN: &str = "jq: ignoring parse error: Truncated value at line 3, column 1\n";
    const GONE: &str = "jq: error: Could not open file @DIR@/gone.seq: No such file or directory\n";
    let both = format!("{WARN}{GONE}");
    #[rustfmt::skip]
    let rows = [
        // The warning is met at the end of `bad`, which is read before the failure.
        (&["bad", "gone"][..], "\u{1e}[[],1]\n\u{1e}[[],2]\n", both.as_str(), 2),
        (&["bad", "gone", "ok"][..], "\u{1e}[[],1]\n\u{1e}[[],2]\n\u{1e}[[],3]\n", both.as_str(), 2),
        (&["mid", "gone", "ok"][..], "\u{1e}[[],1]\n\u{1e}[[],2]\n\u{1e}[[],5]\n\u{1e}[[],3]\n", both.as_str(), 2),
        // The value that takes the reader past the failure is the last one read: a warning after
        // it, in that file or the next, never comes.
        (&["gone", "bad"][..], "\u{1e}[[],1]\n", GONE, 2),
        (&["gone", "d"][..], "\u{1e}[[0],1]\n", GONE, 2),
        (&["gone", "ok", "bad"][..], "\u{1e}[[],3]\n", GONE, 2),
        (&["ok", "gone", "bad"][..], "\u{1e}[[],3]\n\u{1e}[[],4]\n\u{1e}[[],1]\n", GONE, 2),
        // No failure: the warning is all there is.
        (&["bad"][..], "\u{1e}[[],1]\n\u{1e}[[],2]\n", WARN, 0),
    ];
    for (names, stdout, stderr, status) in rows {
        let files: Vec<String> = names.iter().map(|name| format!("{d}/{name}.seq")).collect();
        let mut args: Vec<&str> = vec!["-c", "--seq", "--stream", "."];
        args.extend(files.iter().map(String::as_str));
        let (out, err, code) = run_jq_full(&args, None)?;
        let want_err = stderr.replace("@DIR@", &d);
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (stdout, want_err.as_str(), status),
            "{names:?}"
        );
    }
    Ok(())
}
