//! #6700: under `--seq --stream -s`, a stream that ends in an unfinished term (`Unfinished JSON term
//! at EOF`) with every file that did not open before the last file that has bytes. jq 1.7.1 closed
//! its array before it met the fault there, so it printed the array and never said the warning
//! (#6700 modelled that). jq 1.8.2 reads one more, empty, buffer before it closes the array (#7718):
//! it meets the fault after the failure was reported, so the main loop stops with the array lost, as
//! for a failure after that file (#6481); `succinctly jq` follows it, and those rows are 1.8.2's
//! (`/opt/homebrew/bin/jq`). A last directory keeps jq 1.7.1's reads (#7607). The other rows were
//! captured from jq 1.7.1, which 1.8.2 agrees with.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn an_unfinished_term_at_the_end_after_a_failed_file_loses_the_array_6700() -> Result<()> {
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
        (&["-c", "-s", "--seq", "--stream", "."][..], &["missing.json", "endf.seq"][..], "", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: ignoring parse error: Unfinished JSON term at EOF at line 2, column 4\n", 2),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["good.seq", "missing.json", "endf.seq"][..], "", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: ignoring parse error: Unfinished JSON term at EOF at line 3, column 4\n", 2),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["dir", "endf.seq"][..], "", "jq: error: Is a directory\njq: ignoring parse error: Unfinished JSON term at EOF at line 2, column 4\n", 2),
        (&["-c", "-s", "--seq", "--stream", "length"][..], &["missing.json", "trunc.seq"][..], "", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: ignoring parse error: Unfinished JSON term at EOF at line 1, column 4\n", 2),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["missing.json", "rend.seq"][..], "", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: ignoring parse error: Unfinished JSON term at EOF at line 2, column 4\n", 2),
        (&["-c", "-s", "--seq", "--stream", "empty"][..], &["missing.json", "endf.seq"][..], "", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: ignoring parse error: Unfinished JSON term at EOF at line 2, column 4\n", 2),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["missing.json", "good.seq", "endf.seq"][..], "", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: ignoring parse error: Unfinished JSON term at EOF at line 3, column 4\n", 2),
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
