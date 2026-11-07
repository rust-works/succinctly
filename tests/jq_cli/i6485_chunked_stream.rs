//! #6485: `--stream` reads a file a chunk at a time, and says what jq says of a file that does not
//! open when its reader reaches it, not when the run starts. A fault in the text before it stops
//! the main loop first, so the file is never opened and never reported (exit 5, not 2); a file the
//! program stops short of is not reported either; and a file after the first one that failed is
//! reached only if the value read past the first failure still needs more. Every row was captured
//! from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_chunked_stream_reports_an_unopened_file_when_its_reader_reaches_it_6485() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    for (name, text) in [
        ("t1.json", "[1,"),
        ("bad.json", "[1,}"),
        ("mid.json", "{\"a\":1}\n{\"a\":}\n{\"a\":3}\n"),
        ("nums.json", "1\n2\n3\n"),
        ("bom.json", "\u{feff}[1,}"),
        ("empty.json", ""),
        ("ok.json", "[1,2]\n"),
        ("nonl.json", "7"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    for (flags, names, stdout, stderr, status) in [
        (&["-c", "--stream", "."][..], &["bad", "gone"][..], "[[0],1]\n", "jq: parse error: Expected another key:value pair at line 1, column 4\n", 5),
        (&["-c", "--stream", "."][..], &["mid", "gone", "nums"][..], "[[\"a\"],1]\n[[\"a\"]]\n", "jq: parse error: Missing value in key:value pair at line 2, column 6\n", 5),
        (&["-c", "--stream", "."][..], &["nums", "gone", "bad", "gone2"][..], "[[],1]\n[[],2]\n[[],3]\n[[0],1]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "--stream", "."][..], &["nums", "gone", "mid", "gone2"][..], "[[],1]\n[[],2]\n[[],3]\n[[\"a\"],1]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "--stream", "."][..], &["gone", "t1"][..], "[[0],1]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "--stream", "."][..], &["t1", "gone"][..], "[[0],1]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: parse error: Unfinished JSON term at EOF at line 1, column 3\n", 2),
        (&["-c", "--stream", "."][..], &["nums", "gone"][..], "[[],1]\n[[],2]\n[[],3]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "--stream", "."][..], &["gone", "gone2", "ok"][..], "[[0],1]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/gone2.json: No such file or directory\n", 2),
        (&["-c", "--stream", "."][..], &["ok", "gone", "gone2"][..], "[[0],1]\n[[1],2]\n[[1]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/gone2.json: No such file or directory\n", 2),
        (&["-c", "--stream", "."][..], &["nonl", "gone", "ok"][..], "[[],7]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "--stream", "."][..], &["empty", "gone", "nums"][..], "[[],1]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "--stream", "."][..], &["bom", "gone"][..], "[[0],1]\n", "jq: parse error: Expected another key:value pair at line 1, column 4\n", 5),
        (&["-c", "-n", "--stream", "[inputs]"][..], &["bad", "gone"][..], "", "jq: error (at @DIR@/bad.json:0): Expected another key:value pair at line 1, column 4\n", 5),
        (&["-c", "-n", "--stream", "[inputs]"][..], &["nums", "gone", "ok"][..], "[[[],1],[[],2],[[],3],[[0],1],[[1],2],[[1]]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "-n", "--stream", "first(inputs)"][..], &["nums", "gone"][..], "[[],1]\n", "", 0),
        (&["-c", "-n", "--stream", "input"][..], &["ok", "gone"][..], "[[0],1]\n", "", 0),
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

/// A last file that did not open is a source of its own in jq's reader, so a number at the end of
/// the file before it waits for the read that opens the one that failed; and a directory last is
/// read, and fails the run, even when nothing asks past it.
#[test]
fn a_last_unopened_file_or_directory_is_a_source_of_its_own_6485() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    for (name, text) in [
        ("a1.json", "1"),
        ("two.json", "2 3"),
        ("nonl.json", "[1]"),
        ("nums.json", "1\n2\n3\n"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    std::fs::create_dir(dir.path().join("adir.json"))?;
    for (flags, names, stdout, stderr, status) in [
        (&["-c", "-n", "--stream", "input"][..], &["a1", "gone"][..], "[[],1]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "-n", "--stream", "first(inputs)"][..], &["a1", "gone"][..], "[[],1]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "-n", "--stream", "input,input"][..], &["two", "gone"][..], "[[],2]\n[[],3]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "--stream", "input|halt_error"][..], &["two", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n[[],3]\n", 2),
        (&["-c", "--stream", "."][..], &["a1", "gone"][..], "[[],1]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "--stream", "."][..], &["two", "gone"][..], "[[],2]\n[[],3]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "-n", "--stream", "input,input"][..], &["nonl", "adir"][..], "[[0],1]\n[[0]]\n", "", 2),
        (&["-c", "-n", "--stream", "input"][..], &["a1", "adir"][..], "[[],1]\n", "", 2),
        (&["-c", "-n", "--stream", "first(inputs)"][..], &["nonl", "adir"][..], "[[0],1]\n", "", 0),
        (&["-c", "--stream", "input|halt_error"][..], &["nonl", "adir"][..], "", "[[0]]\n", 2),
        (&["-c", "--stream", "."][..], &["nonl", "adir"][..], "[[0],1]\n[[0]]\n", "", 2),
        (&["-c", "--stream", "."][..], &["adir"][..], "", "", 2),
        (&["-c", "--stream", "."][..], &["nums", "adir", "gone"][..], "[[],1]\n[[],2]\n[[],3]\n", "jq: error: Is a directory\njq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "--stream", "."][..], &["nums", "adir", "nums"][..], "[[],1]\n[[],2]\n[[],3]\n[[],1]\n", "jq: error: Is a directory\n", 2),
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
