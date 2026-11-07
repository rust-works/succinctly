//! #6480: when the last file named did not open, jq's reader goes on to it once the text before it has
//! been read, so a fault at the end of the text is met there: the open error comes first, the fault
//! is located at that file, line 0, and `input_filename` names it. A text that stops at a fault
//! before the file never opens it. Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_fault_at_the_end_of_the_text_is_located_at_the_unopened_last_file_6480() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    for (name, text) in [
        ("t1.json", "[1,"),
        ("op.json", "1 5["),
        ("big.json", "1\n2\n"),
        ("mid.json", "[1,}\n2"),
        ("nums.json", "1\n2\n3\n"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    for (flags, names, stdout, stderr, status) in [
        (&["-nc", "[inputs]|length"][..], &["t1", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0): Unfinished JSON term at EOF at line 1, column 3\n", 2),
        (&["-nc", "try input catch \"c\""][..], &["t1", "gone"][..], "\"c\"\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-nc", "input"][..], &["t1", "gone", "gone2"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/gone2.json: No such file or directory\njq: error (at @DIR@/gone2.json:0): Unfinished JSON term at EOF at line 1, column 3\n", 2),
        (&["-nc", "[inputs]|length"][..], &["op", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0): Unfinished JSON term at EOF at line 1, column 4\n", 2),
        (&["-nc", "[inputs]|length"][..], &["big", "op", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0): Unfinished JSON term at EOF at line 3, column 4\n", 2),
        (&["-nc", "try ([inputs]) catch input_filename"][..], &["t1", "gone"][..], "\"@DIR@/gone.json\"
", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-nc", "[inputs]|length"][..], &["t1", "gone", "nums"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/nums.json:2): Expected separator between values at line 3, column 0\n", 2),
        (&["-nc", "[inputs]|length"][..], &["mid", "gone"][..], "", "jq: error (at @DIR@/mid.json:1): Unmatched '}' at line 1, column 4\n", 5),
        (&["-nc", "try input catch \"c\""][..], &["mid", "gone"][..], "\"c\"\n", "", 0),
        (&["-nc", "input"][..], &["nums", "gone"][..], "1\n", "", 0),
        (&["-c", "."][..], &["t1", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: parse error: Unfinished JSON term at EOF at line 1, column 3\n", 2),
    ] {
        let files: Vec<String> = names.iter().map(|name| format!("{d}/{name}.json")).collect();
        let mut args: Vec<&str> = flags.to_vec();
        args.extend(files.iter().map(String::as_str));
        let (out, err, code) = run_jq_full(&args, None)?;
        let expected_err = stderr.replace("@DIR@", &d);
        let expected_out = stdout.replace("@DIR@", &d);
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (expected_out.as_str(), expected_err.as_str(), status),
            "{flags:?} {names:?}"
        );
    }
    Ok(())
}
