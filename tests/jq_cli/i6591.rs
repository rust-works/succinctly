//! #6591: `--stream-errors`, next to a file that does not open.
//!
//! - jq's main loop stops one event past the first open failure, so a first event that is complete
//!   inside its file (`[1,` has `[[0],1]`, a fault after it notwithstanding) never opens the file
//!   after it; one that is not (`[1`, a bare `1`) waits for the next file, which is read.
//! - The event of a parse error at the end of the text is read at a last file that did not open,
//!   line 0 (`input_filename` is that file), as the JSON reader's fault is (#6480).
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
