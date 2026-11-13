//! #7176: a run that reads `input`/`inputs` over several files, none of which failed to open, no longer
//! walks jq's parser model over every file for a fault to place a failed file by. A fault in the text is still
//! the program's to meet as it reads: these rows are the answers of jq 1.7.1 over the same three files
//! (`f2.json` has a stray `}` in its second line, `f3.json` an array missing a comma), which the change must
//! not move.

use std::process::Command;

use anyhow::Result;

/// (arguments, jq 1.7.1's stdout, the first line of its stderr, its exit status).
#[rustfmt::skip]
const ROWS: &[(&[&str], &str, &str, i32)] = &[
    (&["-c", "-n", "[inputs]"], "", "jq: error (at f2.json:2): Unmatched '}' at line 4, column 1", 5),
    (&["-c", "-n", "first(inputs)"], "{\"a\":1}\n", "", 0),
    (&["-c", ".a"], "1\n2\n3\n", "jq: parse error: Unmatched '}' at line 4, column 1", 5),
    (&["-c", "-n", "-s", "."], "null\n", "", 0),
    (&["-c", "-s", "."], "", "jq: parse error: Unmatched '}' at line 4, column 1", 5),
    (&["-c", "-n", "input, input, input, input, input"], "{\"a\":1}\n{\"a\":2}\n{\"a\":3}\n", "jq: error (at f2.json:2): Unmatched '}' at line 4, column 1", 5),
    (&["-c", "-n", "[limit(2; inputs)]"], "[{\"a\":1},{\"a\":2}]\n", "", 0),
];

#[test]
fn a_fault_in_the_text_is_met_as_the_program_reads_it_with_no_failed_file_7176() -> Result<()> {
    let dir = tempfile::tempdir()?;
    for (name, text) in [
        ("f1.json", "{\"a\":1}\n{\"a\":2}\n"),
        ("f2.json", "{\"a\":3}\n}\n{\"a\":4}\n"),
        ("f3.json", "{\"a\":5}\n[1 2]\n"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    for &(args, stdout, stderr_first, status) in ROWS {
        let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
            .arg("jq")
            .args(args)
            .args(["f1.json", "f2.json", "f3.json"])
            .current_dir(dir.path())
            .output()?;
        let err = String::from_utf8_lossy(&output.stderr);
        let first = err.lines().next().unwrap_or("");
        assert_eq!(
            (
                String::from_utf8_lossy(&output.stdout).as_ref(),
                first,
                output.status.code(),
            ),
            (stdout, stderr_first, Some(status)),
            "{args:?}"
        );
    }
    Ok(())
}
