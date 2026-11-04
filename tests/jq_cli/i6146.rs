//! #6146: where an `input` that runs past the end of a `--seq` stream names its source. jq's rule
//! is the one for plain JSON (#4303): the position is lost (`<unknown>`) when the read that
//! reached the end of input completed the stream's last value, and is the source's newline count
//! otherwise, a stream with no value at all included. `succinctly jq` named the last value's line,
//! or `<stdin>:0` with none. Every row was captured from jq 1.7.1 (`/usr/bin/jq`).
//!
//! Not pinned, because it still differs: a stream that ends in a fault after its last value
//! (`\x1e[1]1`), which is #6137's family.

use anyhow::Result;

use crate::run_jq_full;

const RS: &str = "\u{1e}";

fn past_end(stdin: &str, args: &[&str]) -> Result<String> {
    let mut full = vec!["-c", "--seq"];
    full.extend_from_slice(args);
    let (stdout, stderr, code) = run_jq_full(&full, Some(stdin))?;
    assert_eq!(
        code, 5,
        "{stdin:?} {args:?}: stdout={stdout:?} stderr={stderr:?}"
    );
    Ok(stderr)
}

#[test]
fn test_the_past_end_read_names_the_newline_count_or_loses_it_6146() -> Result<()> {
    for (stdin, args, at) in [
        // The issue's rows: a lone RS after the last value.
        (format!("{RS}1 {RS}"), vec!["[.,input]"], "<unknown>"),
        (
            format!("{RS}1 {RS}"),
            vec!["-n", "[.,input,input]"],
            "<unknown>",
        ),
        // No value at all: the newlines, wherever the RS sits.
        (
            "\n\n\n".to_string() + RS,
            vec!["-n", "[.,input]"],
            "<stdin>:3",
        ),
        (format!("{RS}\n"), vec!["-n", "[input]"], "<stdin>:1"),
        (format!("\n{RS}\n"), vec!["-n", "[input]"], "<stdin>:2"),
        // The last value completed before the final read: the newline count.
        (format!("{RS}1\n"), vec!["[.,input]"], "<stdin>:1"),
        (format!("{RS}1\n\n"), vec!["[.,input]"], "<stdin>:2"),
        (
            format!("{RS}1\n{RS}"),
            vec!["-n", "[input,input,input]"],
            "<stdin>:1",
        ),
        (
            format!("\n{RS}1\n\n\n"),
            vec!["-n", "[input,input,input]"],
            "<stdin>:4",
        ),
        // Completed by the end of input itself: a closing bracket in the last read.
        (format!("{RS}[1]"), vec!["[.,input]"], "<unknown>"),
    ] {
        let stderr = past_end(&stdin, &args)?;
        assert!(
            stderr.contains(&format!("(at {at}): break")),
            "{stdin:?} {args:?}: {stderr:?}"
        );
    }
    Ok(())
}

/// The files are one stream to jq's reader, so the position names the last file and its own
/// newlines, whichever file the last value is in (captured from jq 1.7.1).
#[test]
fn test_several_files_name_the_last_one_6146() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let first = dir.path().join("f1.seq");
    let second = dir.path().join("f2.seq");
    std::fs::write(&first, format!("{RS}1\n"))?;
    std::fs::write(&second, format!("{RS}2 "))?;
    for (a, b, at) in [(&first, &second, "f2.seq:0"), (&second, &first, "f1.seq:1")] {
        let (stdout, stderr, code) = run_jq_full(
            &[
                "-c",
                "--seq",
                "-n",
                "[input,input,input]",
                a.to_str().unwrap(),
                b.to_str().unwrap(),
            ],
            None,
        )?;
        assert_eq!(code, 5, "stdout={stdout:?} stderr={stderr:?}");
        assert!(
            stderr.contains(&format!("{at}): break")),
            "{at}: {stderr:?}"
        );
    }
    Ok(())
}
