//! #6646: `--seq --stream` over a file whose last record is a container with a scalar before its
//! closer (`\x1e[1]`, `\x1e{"a":1}`) and no byte after it. jq's parser holds the closer's event back
//! until the next read, which is the open of the file that follows; the main loop ends there, so
//! the failures after that one are not reported. A closer that follows a container (`[[1]]`), no
//! scalar (`[]`) or a byte after it (`[1]\n`) is read with its own token, and every failure is
//! reported. Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

const RS: &str = "\u{1e}";

const ONE_MISSING: &str = "jq: error: Could not open file @D@/gone: No such file or directory\n";
const TWO_MISSING: &str = "jq: error: Could not open file @D@/gone: No such file or directory\n\
                           jq: error: Could not open file @D@/gone2: No such file or directory\n";

/// `(flag, file text, stdout, stderr)`; each row runs `--seq -c FLAG . FILE gone gone2`.
type Row<'a> = (&'a str, String, String, &'a str);

#[test]
fn a_held_back_closer_ends_the_failures_reached_6646() -> Result<()> {
    let rows: Vec<Row> = vec![
        (
            "--stream",
            format!("{RS}[1]"),
            format!("{RS}[[0],1]\n{RS}[[0]]\n"),
            ONE_MISSING,
        ),
        (
            "--stream-errors",
            format!("{RS}{{\"a\":1}}"),
            format!("{RS}[[\"a\"],1]\n{RS}[[\"a\"]]\n"),
            ONE_MISSING,
        ),
        (
            "--stream",
            format!("{RS}[1]\n{RS}[2]"),
            format!("{RS}[[0],1]\n{RS}[[0]]\n{RS}[[0],2]\n{RS}[[0]]\n"),
            ONE_MISSING,
        ),
        (
            "--stream",
            format!("{RS}[[1]]"),
            format!("{RS}[[0,0],1]\n{RS}[[0,0]]\n{RS}[[0]]\n"),
            TWO_MISSING,
        ),
        (
            "--stream",
            format!("{RS}[1]\n"),
            format!("{RS}[[0],1]\n{RS}[[0]]\n"),
            TWO_MISSING,
        ),
        // A closer that does not match is a fault, not a held-back event.
        (
            "--stream-errors",
            format!("{RS}[1}}"),
            format!(
                "{RS}[\"Unmatched '}}' in the middle of an array at line 1, column 4 (need RS to resync)\",[0]]\n"
            ),
            TWO_MISSING,
        ),
        (
            "--stream",
            format!("{RS}[]"),
            format!("{RS}[[],[]]\n"),
            TWO_MISSING,
        ),
    ];
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    for (flag, text, stdout, stderr) in rows {
        std::fs::write(dir.path().join("f.json"), &text)?;
        let f = format!("{d}/f.json");
        let (gone, gone2) = (format!("{d}/gone"), format!("{d}/gone2"));
        let (out, err, code) = run_jq_full(&["--seq", "-c", flag, ".", &f, &gone, &gone2], None)?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (stdout.as_str(), stderr.replace("@D@", &d).as_str(), 2),
            "{flag} over {text:?}"
        );
    }
    Ok(())
}

/// `(files written first, flags and program, the file names in order, stdout, stderr)`; every run
/// is `jq -c FLAGS... FILES...` and exits 2. `gone` is not there, `dir` and `dir2` are directories.
type StreamRow<'a> = (
    &'a [(&'a str, &'a str)],
    &'a [&'a str],
    &'a [&'a str],
    &'a str,
    &'a str,
);

/// #6646: plain `--stream-errors`, which reads the whole text through the in-memory route, treats a
/// file that did not open in the middle of the list as an empty read, as the chunked `--stream`
/// reader does (#6680): the closer the file before it held back is read there, `input_filename`
/// names it, and the main loop ends at it. Every row was captured from jq 1.7.1.
#[test]
fn stream_errors_reads_a_failed_file_as_an_empty_source_6646() -> Result<()> {
    let rows: &[StreamRow] = &[
        (&[("f.json", "[1,2]")], &["--stream-errors", "."], &["f.json", "dir", "dir2"], "[[0],1]\n[[1],2]\n[[1]]\n", ""),
        (&[("f.json", "[1,2]")], &["--stream-errors", "."], &["f.json", "gone", "gone2"], "[[0],1]\n[[1],2]\n[[1]]\n", "jq: error: Could not open file @D@/gone: No such file or directory\n"),
        (&[("f.json", "[1,2]")], &["--stream-errors", "."], &["f.json", "dir", "g.json"], "[[0],1]\n[[1],2]\n[[1]]\n", ""),
        (&[("f.json", "[1,2]")], &["--stream-errors", "."], &["f.json", "gone", "g.json"], "[[0],1]\n[[1],2]\n[[1]]\n", "jq: error: Could not open file @D@/gone: No such file or directory\n"),
        (&[("f.json", "[1,2]")], &["--stream-errors", "[., input_filename, input_line_number]"], &["f.json", "gone", "g.json"], "[[[0],1],\"@D@/f.json\",0]\n[[[1],2],\"@D@/f.json\",0]\n[[[1]],\"@D@/gone\",0]\n", "jq: error: Could not open file @D@/gone: No such file or directory\n"),
        (&[("f.json", "1")], &["--stream-errors", "[., input_filename, input_line_number]"], &["f.json", "gone"], "[[[],1],\"@D@/gone\",0]\n", "jq: error: Could not open file @D@/gone: No such file or directory\n"),
        (&[("f.json", "[1,2]\n")], &["--stream-errors", "."], &["f.json", "gone", "gone2"], "[[0],1]\n[[1],2]\n[[1]]\n", "jq: error: Could not open file @D@/gone: No such file or directory\njq: error: Could not open file @D@/gone2: No such file or directory\n"),
        (&[("f.json", "[1,2]"), ("e.json", "")], &["--stream-errors", "."], &["f.json", "e.json", "gone", "gone2"], "[[0],1]\n[[1],2]\n[[1]]\n", "jq: error: Could not open file @D@/gone: No such file or directory\njq: error: Could not open file @D@/gone2: No such file or directory\n"),
        (&[("f.json", "[1,2]"), ("e.json", "")], &["--stream-errors", "."], &["f.json", "e.json", "dir", "g.json"], "[[0],1]\n[[1],2]\n[[1]]\n[[\"a\"],1]\n", "jq: error: Is a directory\n"),
        (&[("f.json", "[1,2]")], &["--stream-errors", "first(inputs)"], &["f.json", "dir", "g.json"], "[[1],2]\n[[\"a\"],1]\n", "jq: error: Is a directory\n"),
        (&[("f.json", "[1,2]\n")], &["--stream-errors", "."], &["f.json", "dir", "g.json"], "[[0],1]\n[[1],2]\n[[1]]\n[[\"a\"],1]\n", "jq: error: Is a directory\n"),
    ];
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::create_dir(dir.path().join("dir"))?;
    std::fs::create_dir(dir.path().join("dir2"))?;
    std::fs::write(dir.path().join("g.json"), "{\"a\":1}")?;
    for (files, flags, order, stdout, stderr) in rows {
        for (name, text) in *files {
            std::fs::write(dir.path().join(name), text)?;
        }
        let mut args: Vec<String> = vec!["-c".into()];
        args.extend(flags.iter().map(|flag| (*flag).to_string()));
        args.extend(order.iter().map(|name| format!("{d}/{name}")));
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let (out, err, code) = run_jq_full(&args, None)?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (
                stdout.replace("@D@", &d).as_str(),
                stderr.replace("@D@", &d).as_str(),
                2
            ),
            "{flags:?} over {order:?}"
        );
    }
    Ok(())
}

/// #6646: the read that follows a file's held-back closer is the next file's, even an empty one, so
/// the failures behind that file are met by later reads and all reported (`--seq`, jq 1.7.1).
#[test]
fn an_empty_file_takes_the_held_back_closer_6646() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::write(dir.path().join("f.json"), format!("{RS}[1]"))?;
    std::fs::write(dir.path().join("e.json"), "")?;
    for flag in ["--stream", "--stream-errors"] {
        let files = [
            format!("{d}/f.json"),
            format!("{d}/e.json"),
            format!("{d}/gone"),
            format!("{d}/gone2"),
        ];
        let mut args = vec!["--seq", "-c", flag, "."];
        args.extend(files.iter().map(String::as_str));
        let (out, err, code) = run_jq_full(&args, None)?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (
                format!("{RS}[[0],1]\n{RS}[[0]]\n").as_str(),
                TWO_MISSING.replace("@D@", &d).as_str(),
                2
            ),
            "{flag}"
        );
    }
    Ok(())
}
