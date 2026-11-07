//! #6481: a slurped `--stream` or `--seq` input whose stream ends in a fault, with a file that does not
//! open. jq's reader moves on to that file in the call that consumes the last byte, so the fault is
//! that call's result and the main loop, which reads no further once a file has failed, never gets
//! to the array: stdout is empty and the run exits 2. Without a file that failed, the array comes out
//! first and the fault after it. Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_slurped_stream_ending_in_a_fault_loses_its_array_beside_an_unopened_file_6481() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    for (name, text) in [
        ("t1.json", "[1,"),
        ("t4.json", "\"x\"\n[1,"),
        ("nums.json", "1\n2\n3\n"),
        ("s1.json", "\u{1e}[1]\n\u{1e}[2,"),
        ("s2.json", "\u{1e}[1]\n\u{1e}[2]\n"),
        ("s3.json", "\u{1e}[3"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    std::fs::create_dir(dir.path().join("adir.json"))?;
    for (flags, names, stdout, stderr, status) in [
        (&["-c", "-s", "--stream", "."][..], &["t1", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: parse error: Unfinished JSON term at EOF at line 1, column 3\n", 2),
        (&["-c", "-s", "--stream", "."][..], &["t4", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: parse error: Unfinished JSON term at EOF at line 2, column 3\n", 2),
        (&["-c", "-s", "--stream", "."][..], &["t1", "gone", "nums"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: parse error: Expected separator between values at line 3, column 0\n", 2),
        (&["-c", "-s", "--stream", "."][..], &["t1", "adir"][..], "", "jq: parse error: Unfinished JSON term at EOF at line 1, column 3\n", 2),
        (&["-c", "-s", "--stream", "."][..], &["nums", "gone"][..], "[[[],1],[[],2],[[],3]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "-s", "--stream", "."][..], &["t1"][..], "[[[0],1]]\n", "jq: parse error: Unfinished JSON term at EOF at line 1, column 3\n", 5),
        (&["-c", "-s", "--seq", "."][..], &["t1", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: ignoring parse error: Unfinished abandoned text at EOF at line 1, column 3\n", 2),
        (&["-c", "-s", "--seq", "."][..], &["gone", "t1"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: ignoring parse error: Unfinished abandoned text at EOF at line 1, column 3\n", 2),
        (&["-c", "-s", "--seq", "."][..], &["t4", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: ignoring parse error: Unfinished abandoned text at EOF at line 2, column 3\n", 2),
        (&["-c", "-s", "--seq", "."][..], &["s3", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: ignoring parse error: Unfinished JSON term at EOF at line 1, column 3\n", 2),
        (&["-c", "-s", "--seq", "."][..], &["s2", "s3", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: ignoring parse error: Unfinished JSON term at EOF at line 3, column 3\n", 2),
        (&["-c", "-s", "--seq", "."][..], &["s2", "gone"][..], "\u{1e}[[1],[2]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "-s", "--seq", "."][..], &["gone", "s2"][..], "\u{1e}[[1],[2]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "-s", "--seq", "."][..], &["s2", "adir"][..], "\u{1e}[[1],[2]]\n", "", 2),
        (&["-c", "-s", "--seq", "."][..], &["t1", "adir"][..], "", "jq: ignoring parse error: Unfinished abandoned text at EOF at line 1, column 3\n", 2),
        (&["-c", "-s", "--seq", "."][..], &["t1"][..], "\u{1e}[]\n", "jq: ignoring parse error: Unfinished abandoned text at EOF at line 1, column 3\n", 0),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["t1", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: ignoring parse error: Unfinished abandoned text at EOF at line 1, column 3\n", 2),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["s1", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: ignoring parse error: Unfinished JSON term at EOF at line 2, column 4\n", 2),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["s2", "s3", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: ignoring parse error: Unfinished JSON term at EOF at line 3, column 3\n", 2),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["s2", "gone"][..], "\u{1e}[[[0],1],[[0]],[[0],2],[[0]]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["s3"][..], "\u{1e}[]\n", "jq: ignoring parse error: Unfinished JSON term at EOF at line 1, column 3\n", 0),
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
