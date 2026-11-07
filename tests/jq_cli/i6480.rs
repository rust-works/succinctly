//! #6480: under `-n` a file that does not open is reported when a read reaches it (#6475). A text
//! that ends inside a value (`[1,`, `{"a":`, `1 [`) faults at its end, and jq's parser asks the
//! reader for more before it gives up, which opens the next file: the read that raises
//! `Unfinished JSON term at EOF` reports every file after the last one that opened. #6484 left
//! that read without the report, so the run ended in the fault and exited 5, or 0 under `try`.
//! Every row was captured from jq 1.7.1.
//!
//! The `(at ...)` marker of the fault names the last file named, at line 0: jq's reader is there
//! when its parser gives up (#6480's step 5).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_fault_at_the_end_of_the_text_reports_the_files_after_it_under_null_input_6480() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    for (name, text) in [
        ("open", "["),
        ("comma", "[1,"),
        ("key", "{\"a\":"),
        ("nl", "[1,\n"),
        ("small", "1 5["),
        ("ustr", "\"abc"),
        ("bom", "\u{feff}[1,"),
        ("num", "12"),
        ("empty", ""),
    ] {
        std::fs::write(dir.path().join(format!("{name}.json")), text)?;
    }
    std::fs::create_dir(dir.path().join("adir"))?;
    let gone = |name: &str| {
        format!("jq: error: Could not open file @DIR@/{name}.json: No such file or directory\n")
    };
    let fault = |what: &str, at: &str| format!("jq: error (at @AT@): {what} at EOF at line {at}\n");
    let term = |at: &str| fault("Unfinished JSON term", at);
    for (program, names, stdout, stderr, status) in [
        (
            "[inputs]|length",
            &["open", "gone"][..],
            "",
            gone("gone") + &term("1, column 1"),
            2,
        ),
        (
            "[inputs]|length",
            &["comma", "gone"][..],
            "",
            gone("gone") + &term("1, column 3"),
            2,
        ),
        (
            "[inputs]|length",
            &["key", "gone"][..],
            "",
            gone("gone") + &term("1, column 5"),
            2,
        ),
        (
            "[inputs]|length",
            &["nl", "gone"][..],
            "",
            gone("gone") + &term("2, column 0"),
            2,
        ),
        (
            "[inputs]|length",
            &["small", "gone"][..],
            "",
            gone("gone") + &term("1, column 4"),
            2,
        ),
        (
            "[inputs]|length",
            &["bom", "gone"][..],
            "",
            gone("gone") + &term("1, column 3"),
            2,
        ),
        (
            "[inputs]|length",
            &["ustr", "gone"][..],
            "",
            gone("gone") + &fault("Unfinished string", "1, column 4"),
            2,
        ),
        (
            "try input catch \"c\"",
            &["comma", "gone"][..],
            "\"c\"\n",
            gone("gone"),
            2,
        ),
        (
            "try input catch \"c\"",
            &["nl", "gone"][..],
            "\"c\"\n",
            gone("gone"),
            2,
        ),
        (
            "input",
            &["comma", "m1", "m2"][..],
            "",
            gone("m1") + &gone("m2") + &term("1, column 3"),
            2,
        ),
        (
            "[inputs]",
            &["comma", "empty", "gone"][..],
            "",
            gone("gone") + &term("1, column 3"),
            2,
        ),
        // A directory last is a failure jq never words, since it never asks past the last file,
        // but it still makes the run exit 2.
        (
            "[inputs]|length",
            &["comma", "adir"][..],
            "",
            term("1, column 3"),
            2,
        ),
        (
            "try input catch \"c\"",
            &["comma", "adir"][..],
            "\"c\"\n",
            String::new(),
            2,
        ),
        // The reads before the fault do not reach the end: `1` and `5` are complete at the space
        // and the `[` after them.
        ("input", &["small", "gone"][..], "1\n", String::new(), 0),
        (
            "input, input",
            &["small", "gone"][..],
            "1\n5\n",
            String::new(),
            0,
        ),
        (
            "input, input, input",
            &["small", "gone"][..],
            "1\n5\n",
            gone("gone") + &term("1, column 4"),
            2,
        ),
        (
            "first(inputs)",
            &["num", "comma", "gone"][..],
            "12\n",
            String::new(),
            0,
        ),
        // A file that does not open between two that do is reached before the fault.
        (
            "input",
            &["comma", "m1", "num"][..],
            "",
            gone("m1") + &term("1, column 5"),
            2,
        ),
    ] {
        let paths: Vec<String> = names
            .iter()
            .map(|&name| match name {
                "adir" => format!("{d}/adir"),
                name => format!("{d}/{name}.json"),
            })
            .collect();
        let mut args = vec!["-c", "-n", program];
        args.extend(paths.iter().map(String::as_str));
        let (out, err, code) = run_jq_full(&args, None)?;
        // The fault is raised at the last file named, line 0.
        let at = format!("{}:0", paths.last().map_or("", String::as_str));
        let expected_err = stderr.replace("@AT@", &at).replace("@DIR@", &d);
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (stdout, expected_err.as_str(), status),
            "{program} {names:?}"
        );
    }
    Ok(())
}
