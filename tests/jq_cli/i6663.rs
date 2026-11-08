//! #6663: a text that faults and ends in a bare number (`3` of `1 }`, `2 ]`, `3`, with no newline
//! after it) is read by a program that reads `input` one fault at a time, and jq's reader goes on past
//! each fault to the files after the text: a missing file is reported, a directory is, and the run
//! exits 2. succinctly cut its report at the first fault, as it must for the main loop's own read of
//! one (`.` stops there, exit 5). Every expected string was captured from `/usr/bin/jq` 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

const MULTI_1: &str = "jq: error (at @DIR@/multi.json:1): Unmatched '}' at line 1, column 3\n";
const MULTI_2: &str = "jq: error (at @DIR@/multi.json:2): Unmatched ']' at line 1, column 6\n";
const TWO_1: &str = "jq: error (at @DIR@/two.json:1): Unmatched '}' at line 1, column 3\n";
const GONE: &str = "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n";
const GONE2: &str = "jq: error: Could not open file @DIR@/gone2.json: No such file or directory\n";
const DIRECTORY: &str = "jq: error: Is a directory\n";
const BREAK: &str = "jq: error (at <unknown>): break\n";

#[test]
fn a_program_reading_past_faults_reaches_the_files_after_them_6663() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    // The last token is a bare number, so the text does not end where a value does.
    std::fs::write(dir.path().join("multi.json"), "1 }\n2 ]\n3")?;
    std::fs::write(dir.path().join("two.json"), "1 }\n2")?;
    // The same text with a newline after the number ends where its value does.
    std::fs::write(dir.path().join("multin.json"), "1 }\n2 ]\n3\n")?;
    std::fs::create_dir(dir.path().join("dir"))?;
    let expand = |text: &str| text.replace("@DIR@", &d);

    // (program, files, stdout, stderr, exit status).
    for (program, names, stdout, stderr, code) in [
        (
            "input",
            &["multi.json", "gone.json", "dir"][..],
            "",
            [MULTI_1, MULTI_2, GONE, DIRECTORY, BREAK].concat(),
            2,
        ),
        (
            "[inputs]",
            &["multi.json", "gone.json", "gone2.json"][..],
            "[]\n",
            [MULTI_1, MULTI_2, GONE, GONE2].concat(),
            2,
        ),
        (
            "first(inputs)",
            &["two.json", "gone.json", "gone2.json"][..],
            "",
            [TWO_1, GONE, GONE2].concat(),
            2,
        ),
        (
            "[.,input]",
            &["two.json", "gone.json", "dir"][..],
            "",
            [TWO_1, GONE, DIRECTORY, BREAK].concat(),
            2,
        ),
        // The main loop's own read of a fault stops the run before any of them.
        (
            ".",
            &["multi.json", "gone.json", "dir"][..],
            "1\n",
            "jq: parse error: Unmatched '}' at line 1, column 3\n".to_string(),
            5,
        ),
        // The text with a newline after its number already agreed.
        (
            "[inputs]",
            &["multin.json", "gone.json", "dir"][..],
            "[]\n",
            [
                "jq: error (at @DIR@/multin.json:1): Unmatched '}' at line 1, column 3\n",
                "jq: error (at @DIR@/multin.json:2): Unmatched ']' at line 1, column 6\n",
                GONE,
            ]
            .concat(),
            2,
        ),
    ] {
        let files: Vec<String> = names.iter().map(|name| format!("{d}/{name}")).collect();
        let mut args = vec!["-c", program];
        args.extend(files.iter().map(String::as_str));
        let (out, err, status) = run_jq_full(&args, None)?;
        assert_eq!(out, stdout, "{program} {names:?} stdout");
        assert_eq!(err, expand(&stderr), "{program} {names:?} stderr");
        assert_eq!(status, code, "{program} {names:?} status");
    }
    Ok(())
}
