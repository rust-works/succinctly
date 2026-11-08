//! #6588: past a last file that did not open, jq's `--seq` reader asks that file for more before
//! it is at its end, so the refill that ends the stream is the empty one at the missing file. The
//! last opened file's last chunk is then not the final buffer, and #2998's silent end, which drops
//! a record a final buffer starting on an RS holds, does not apply there. Every row was captured
//! from jq 1.7.1; the ones without the missing file show the drop still applying.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_seq_final_buffer_drop_does_not_apply_before_an_unopened_last_file_6588() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    for (name, text) in [
        ("seq.json", "\u{1e}1\n\u{1e}2\n"),
        ("nonl.json", "\u{1e}1\n\u{1e}[2]"),
        ("rsend.json", "\u{1e}[1]\n\u{1e}"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    #[rustfmt::skip]
    let rows = [
        (&["--seq", "-c", "-n", "[inputs]"][..], &["nonl", "gone"][..], "\u{1e}[1,[2]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--seq", "-c", "-n", "[inputs]"][..], &["nonl"][..], "\u{1e}[1]\n", "", 0),
        (&["--seq", "-c", "."][..], &["nonl", "gone"][..], "\u{1e}1\n\u{1e}[2]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--seq", "-c", "."][..], &["nonl"][..], "\u{1e}1\n", "", 0),
        (&["--seq", "-c", "-n", "[inputs]"][..], &["rsend", "gone"][..], "\u{1e}[[1]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--seq", "-c", "[., input]"][..], &["seq", "rsend", "gone"][..], "\u{1e}[1,2]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0): break\n", 2),
        (&["--seq", "-c", "[., input]"][..], &["seq", "rsend"][..], "\u{1e}[1,2]\n", "jq: error (at @DIR@/rsend.json:1): break\n", 5),
        (&["--seq", "-c", "[., input_filename, input_line_number]"][..], &["nonl", "gone"][..], "\u{1e}[1,\"@DIR@/nonl.json\",1]\n\u{1e}[[2],\"@DIR@/nonl.json\",1]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["--seq", "-s", "-c", "."][..], &["nonl", "gone"][..], "\u{1e}[1,[2]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
    ];
    for (flags, names, stdout, stderr, status) in rows {
        let files: Vec<String> = names
            .iter()
            .map(|name| format!("{d}/{name}.json"))
            .collect();
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
