//! #6485: jq 1.7.1's `-s --stream` read the fault a stream ends in in a call after the array, and
//! its main loop makes no further call once a file has failed to open: with a file that did not open
//! ahead of the faulting text the array was printed and the fault never was, and the run exited 2.
//! jq 1.8.2 reads the closing refill before it closes the array, so it meets the fault first and
//! prints it alone, which `succinctly jq` follows since #7807: those rows were captured from jq 1.8.2
//! (`/opt/homebrew/bin/jq`) and say what 1.7.1 printed. The others were captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_fault_read_after_the_slurped_array_is_not_reached_after_a_failure_6485() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    for (name, text) in [
        ("t1.json", "[1,"),
        ("n1.json", "1"),
        ("a2.json", "[1]\n[2,"),
        ("nums.json", "1\n2\n3\n"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    for (flags, names, stdout, stderr, status) in [
        // jq 1.7.1: `[[[0],1]]` and no parse error.
        (&["-c", "-s", "--stream", "."][..], &["gone", "t1"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: parse error: Unfinished JSON term at EOF at line 1, column 3\n", 2),
        // jq 1.7.1: `[[[0],1],[[0]],[[0],2]]` and no parse error.
        (&["-c", "-s", "--stream", "."][..], &["gone", "a2"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: parse error: Unfinished JSON term at EOF at line 2, column 3\n", 2),
        (&["-c", "-s", "--stream", "."][..], &["gone", "n1"][..], "[[[],1]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        // jq 1.7.1: `[[[],1],[[],2],[[],3],[[0],1]]` and no parse error.
        (&["-c", "-s", "--stream", "."][..], &["nums", "gone", "t1"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: parse error: Unfinished JSON term at EOF at line 4, column 3\n", 2),
        (&["-c", "-s", "--stream", "."][..], &["gone", "t1", "gone2"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/gone2.json: No such file or directory\njq: parse error: Unfinished JSON term at EOF at line 1, column 3\n", 2),
        // jq 1.7.1: `[[[0],1]]` first.
        (&["-c", "-s", "--stream", "."][..], &["t1"][..], "", "jq: parse error: Unfinished JSON term at EOF at line 1, column 3\n", 5),
        (&["-c", "-s", "--stream", "."][..], &["n1"][..], "[[[],1]]\n", "", 0),
        (&["-c", "-s", "--stream", "."][..], &["gone", "nums"][..], "[[[],1],[[],2],[[],3]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
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
