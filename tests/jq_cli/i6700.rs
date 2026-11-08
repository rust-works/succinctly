//! #6700: under `--seq --stream -s`, a stream that ends in an unfinished term (`Unfinished JSON term
//! at EOF`) with every file that did not open before the last file that has bytes is read through to
//! its end before the failure is met again: jq prints the array and never says the warning. With a
//! failure after that file (#6481), or none, the warning is said and the array lost or printed as
//! before, and an RS-only end is the other warning. Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn an_unfinished_term_at_the_end_after_a_failed_file_keeps_the_array_and_says_nothing_6700(
) -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::create_dir(dir.path().join("dir"))?;
    for (name, text) in [
        ("good.seq", "\u{1e}1\n"),
        ("endf.seq", "\u{1e}1\n\u{1e}[1,"),
        ("trunc.seq", "\u{1e}[1,"),
        ("endrs.seq", "\u{1e}1\n\u{1e}"),
        ("rend.seq", "\u{1e}12\n\u{1e}[1,"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    #[rustfmt::skip]
    let rows = [
        (&["-c", "-s", "--seq", "--stream", "."][..], &["missing.json", "endf.seq"][..], "\u{1e}[[[],1],[[0],1]]\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["good.seq", "missing.json", "endf.seq"][..], "\u{1e}[[[],1],[[],1],[[0],1]]\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["dir", "endf.seq"][..], "\u{1e}[[[],1],[[0],1]]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-s", "--seq", "--stream", "length"][..], &["missing.json", "trunc.seq"][..], "\u{1e}1\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["missing.json", "rend.seq"][..], "\u{1e}[[[],12],[[0],1]]\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
        (&["-c", "-s", "--seq", "--stream", "empty"][..], &["missing.json", "endf.seq"][..], "", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["missing.json", "good.seq", "endf.seq"][..], "\u{1e}[[[],1],[[],1],[[0],1]]\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["endf.seq"][..], "\u{1e}[[[],1],[[0],1]]\n", "jq: ignoring parse error: Unfinished JSON term at EOF at line 2, column 4\n", 0),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["endf.seq", "missing.json"][..], "", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: ignoring parse error: Unfinished JSON term at EOF at line 2, column 4\n", 2),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["endf.seq", "dir"][..], "", "jq: ignoring parse error: Unfinished JSON term at EOF at line 2, column 4\n", 2),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["missing.json", "endrs.seq"][..], "\u{1e}[[[],1]]\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
    ];
    for (flags, names, stdout, stderr, status) in rows {
        let files: Vec<String> = names.iter().map(|name| format!("{d}/{name}")).collect();
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
