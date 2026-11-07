//! #6476: a text that ends inside a value, followed by a file that does not open. jq's parser meets
//! the end-of-text fault after its reader has moved on to the files that would not open, so it
//! reports them first, then the fault, and exits 2 whatever else happened (`state->failures`). Every
//! row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn an_unopened_file_after_a_text_that_ends_inside_a_value_is_reported_first_6476() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    for (name, text) in [
        ("t1.json", "[1,"),
        ("t4.json", "\"x\"\n[1,"),
        ("big.json", "1\n2\n"),
        ("nums.json", "1\n2\n3\n"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    let path = |name: &str| format!("{d}/{name}.json");
    for (flags, names, stdout, stderr, status) in [
        (&["."][..], &["t1", "gone"][..], "", "jq: error: Could not open file {d}/gone.json: No such file or directory\njq: parse error: Unfinished JSON term at EOF at line 1, column 3\n", 2),
        (&["-c", "."][..], &["t4", "gone"][..], "\"x\"\n", "jq: error: Could not open file {d}/gone.json: No such file or directory\njq: parse error: Unfinished JSON term at EOF at line 2, column 3\n", 2),
        (&["-c", "."][..], &["big", "t1", "gone", "gone2"][..], "1\n2\n", "jq: error: Could not open file {d}/gone.json: No such file or directory\njq: error: Could not open file {d}/gone2.json: No such file or directory\njq: parse error: Unfinished JSON term at EOF at line 3, column 3\n", 2),
        (&["-c", "."][..], &["t1", "gone", "nums"][..], "", "jq: error: Could not open file {d}/gone.json: No such file or directory\njq: parse error: Expected separator between values at line 3, column 0\n", 2),
        (&["-s", "-c", "."][..], &["t1", "gone"][..], "", "jq: error: Could not open file {d}/gone.json: No such file or directory\njq: parse error: Unfinished JSON term at EOF at line 1, column 3\n", 2),
        (&["-s", "-c", "."][..], &["t4", "gone"][..], "", "jq: error: Could not open file {d}/gone.json: No such file or directory\njq: parse error: Unfinished JSON term at EOF at line 2, column 3\n", 2),
        (&["-s", "-c", "."][..], &["t1", "gone", "nums"][..], "", "jq: error: Could not open file {d}/gone.json: No such file or directory\njq: parse error: Expected separator between values at line 3, column 0\n", 2),
        (&["-s", "-c", "."][..], &["big", "gone", "t1"][..], "", "jq: error: Could not open file {d}/gone.json: No such file or directory\njq: parse error: Unfinished JSON term at EOF at line 3, column 3\n", 2),
        (&["-c", "--stream", "."][..], &["t1", "gone"][..], "[[0],1]\n", "jq: error: Could not open file {d}/gone.json: No such file or directory\njq: parse error: Unfinished JSON term at EOF at line 1, column 3\n", 2),
        (&["-c", "--seq", "."][..], &["t1", "gone"][..], "", "jq: error: Could not open file {d}/gone.json: No such file or directory\njq: ignoring parse error: Unfinished abandoned text at EOF at line 1, column 3\n", 2),
        (&["-c", "."][..], &["nums", "gone"][..], "1\n2\n3\n", "jq: error: Could not open file {d}/gone.json: No such file or directory\n", 2),
        (&["-s", "-c", "."][..], &["nums", "gone"][..], "[1,2,3]\n", "jq: error: Could not open file {d}/gone.json: No such file or directory\n", 2),
    ] {
        let files: Vec<String> = names.iter().map(|name| path(name)).collect();
        let mut args: Vec<&str> = flags.to_vec();
        args.extend(files.iter().map(String::as_str));
        let (out, err, code) = run_jq_full(&args, None)?;
        let expected_err = stderr.replace("{d}", &d);
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (stdout, expected_err.as_str(), status),
            "{flags:?} {names:?}"
        );
    }
    Ok(())
}
