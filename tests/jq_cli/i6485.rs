//! #6485: a file that does not open is reported when jq's reader reaches it, so a fault in the text
//! before it stops the main loop first: the file is never opened, never reported, and the run exits 5
//! rather than 2. Under `-s` the stream is read whole before the program runs, which the up-front
//! report of the open failures ignored. Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_fault_before_an_unopened_file_stops_the_run_before_it_under_slurp_6485() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    for (name, text) in [
        ("t1.json", "[1,"),
        ("bad.json", "[1,}"),
        ("mid.json", "{\"a\":1}\n{\"a\":}\n{\"a\":3}\n"),
        ("nums.json", "1\n2\n3\n"),
        ("bom.json", "\u{feff}[1,}"),
        ("empty.json", ""),
        ("a.json", "[1,"),
        ("b.json", "2]"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    for (flags, names, stdout, stderr, status) in [
        (&["-c", "-s", "."][..], &["bad", "gone"][..], "", "jq: parse error: Unmatched '}' at line 1, column 4\n", 5),
        (&["-c", "-s", "."][..], &["mid", "gone"][..], "", "jq: parse error: Unmatched '}' at line 2, column 6\n", 5),
        (&["-c", "-s", "."][..], &["bad", "gone", "nums"][..], "", "jq: parse error: Unmatched '}' at line 1, column 4\n", 5),
        (&["-c", "-s", "."][..], &["mid", "gone", "nums"][..], "", "jq: parse error: Unmatched '}' at line 2, column 6\n", 5),
        (&["-c", "-s", "."][..], &["nums", "gone", "bad"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: parse error: Unmatched '}' at line 4, column 4\n", 2),
        (&["-c", "-s", "."][..], &["gone", "bad"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: parse error: Unmatched '}' at line 1, column 4\n", 2),
        (&["-c", "-s", "."][..], &["nums", "gone", "mid", "gone2"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: parse error: Unmatched '}' at line 5, column 6\n", 2),
        (&["-c", "-s", "."][..], &["nums", "gone"][..], "[1,2,3]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "-s", "."][..], &["t1", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: parse error: Unfinished JSON term at EOF at line 1, column 3\n", 2),
        (&["-c", "-s", "."][..], &["bom", "gone"][..], "", "jq: parse error: Unmatched '}' at line 1, column 4\n", 5),
        (&["-c", "-s", "."][..], &["nums", "bad", "gone"][..], "", "jq: parse error: Unmatched '}' at line 4, column 4\n", 5),
        (&["-c", "-s", "--stream", "."][..], &["bad", "gone"][..], "", "jq: parse error: Expected another key:value pair at line 1, column 4\n", 5),
        (&["-c", "-s", "--stream", "."][..], &["mid", "gone"][..], "", "jq: parse error: Missing value in key:value pair at line 2, column 6\n", 5),
        (&["-c", "-s", "--stream", "."][..], &["bad", "gone", "nums"][..], "", "jq: parse error: Expected another key:value pair at line 1, column 4\n", 5),
        (&["-c", "-s", "--stream", "."][..], &["mid", "gone", "nums"][..], "", "jq: parse error: Missing value in key:value pair at line 2, column 6\n", 5),
        (&["-c", "-s", "--stream", "."][..], &["nums", "gone", "bad"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: parse error: Expected another key:value pair at line 4, column 4\n", 2),
        (&["-c", "-s", "--stream", "."][..], &["gone", "bad"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: parse error: Expected another key:value pair at line 1, column 4\n", 2),
        (&["-c", "-s", "--stream", "."][..], &["nums", "gone", "mid", "gone2"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: parse error: Missing value in key:value pair at line 5, column 6\n", 2),
        (&["-c", "-s", "--stream", "."][..], &["nums", "gone"][..], "[[[],1],[[],2],[[],3]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "-s", "--stream", "."][..], &["t1", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: parse error: Unfinished JSON term at EOF at line 1, column 3\n", 2),
        (&["-c", "-s", "--stream", "."][..], &["bom", "gone"][..], "", "jq: parse error: Expected another key:value pair at line 1, column 4\n", 5),
        (&["-c", "-s", "--stream", "."][..], &["nums", "bad", "gone"][..], "", "jq: parse error: Expected another key:value pair at line 4, column 4\n", 5),
        (&["-c", "-s", "."][..], &["empty", "bom", "gone"][..], "", "jq: parse error: Unmatched '}' at line 1, column 4\n", 5),
        (&["-c", "-s", "."][..], &["a", "b", "gone"][..], "[[1,2]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "-s", "--stream", "."][..], &["empty", "bom", "gone"][..], "", "jq: parse error: Expected another key:value pair at line 1, column 4\n", 5),
        (&["-c", "-n", "[inputs]"][..], &["bad", "gone"][..], "", "jq: error (at @DIR@/bad.json:0): Unmatched '}' at line 1, column 4\n", 5),
        (&["-c", "-n", "[inputs]"][..], &["mid", "gone", "nums"][..], "", "jq: error (at @DIR@/mid.json:2): Unmatched '}' at line 2, column 6\n", 5),
        (&["-c", "-n", "input"][..], &["bad", "gone"][..], "", "jq: error (at @DIR@/bad.json:0): Unmatched '}' at line 1, column 4\n", 5),
        (&["-c", "-s", "-R", "."][..], &["bad", "gone"][..], "\"[1,}\"\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "-s", "--seq", "."][..], &["bad", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: ignoring parse error: Unfinished abandoned text at EOF at line 1, column 4\n", 2),
        (&["-c", "-n", "-s", "."][..], &["bad", "gone"][..], "null\n", "", 0),
    ] {
        let files: Vec<String> = names.iter().map(|name| format!("{d}/{name}.json")).collect();
        let mut args: Vec<&str> = flags.to_vec();
        args.extend(files.iter().map(String::as_str));
        let (out, err, code) = run_jq_full(&args, None)?;
        let expected_err = stderr.replace("@DIR@", &d);
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (stdout, expected_err.as_str(), status),
            "{flags:?} {names:?}"
        );
    }
    Ok(())
}
