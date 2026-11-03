//! #6023: an input file named `-` is standard input, as in jq. It is read once (a second `-` is
//! empty), `input_filename` is `"<stdin>"` for it, and it composes with real files, `-s`, `-R`,
//! `--stream`, `--seq`, `-n` with `input`/`inputs`, a byte-order mark and a missing final newline.
//!
//! The rows are `tests/data/jq-dash-stdin-oracle-6023.tsv`: arguments (`@A` and `@B` are two real
//! files, written here), stdin, stdout and exit status, captured from `/usr/bin/jq` 1.7.1. Rows whose
//! output names a file are left out, since the files' paths differ per run.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_dash_input_file_reads_stdin_as_jq_does() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let a = dir.path().join("A.json");
    let b = dir.path().join("B.json");
    std::fs::write(&a, "{\"a\":1,\"b\":[1,2]}\n")?;
    std::fs::write(&b, "[1] 2\n")?;
    let table = include_str!("../data/jq-dash-stdin-oracle-6023.tsv");
    let mut rows = 0;
    for line in table
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
    {
        let cols: Vec<&str> = line.split('\t').collect();
        assert_eq!(cols.len(), 4, "{line}");
        let args: Vec<String> = serde_json::from_str(cols[0])?;
        let stdin: String = serde_json::from_str(cols[1])?;
        let want_out: String = serde_json::from_str(cols[2])?;
        let want_code: i32 = cols[3].parse()?;
        let args: Vec<String> = args
            .into_iter()
            .map(|arg| match arg.as_str() {
                "@A" => a.to_string_lossy().into_owned(),
                "@B" => b.to_string_lossy().into_owned(),
                _ => arg,
            })
            .collect();
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let (out, _, code) = run_jq_full(&refs, Some(&stdin))?;
        assert_eq!(
            (out.as_str(), code),
            (want_out.as_str(), want_code),
            "jq {line}"
        );
        rows += 1;
    }
    assert!(rows > 400, "{rows} rows");
    Ok(())
}

/// `--slurpfile`, `--rawfile` and `-f` do not take `-` for stdin in jq either: they fail to open it.
#[test]
fn the_other_file_options_do_not_read_stdin_for_a_dash() -> Result<()> {
    for args in [
        &["-nc", "--slurpfile", "x", "-", "$x"][..],
        &["-nc", "--rawfile", "x", "-", "$x"][..],
        &["-c", "-f", "-"][..],
    ] {
        let (out, _, code) = run_jq_full(args, Some("1 2"))?;
        assert_eq!((out.as_str(), code), ("", 2), "{args:?}");
    }
    Ok(())
}

/// A route that reads the file list, declines it and leaves the next route to read it again must
/// find stdin's bytes still there: `--seq` on a stream with no RS byte declines the plain route,
/// and the warning's position came out as column 0 when the bytes had gone. The text is jq's.
#[test]
fn stdin_given_as_a_dash_survives_a_route_that_declines_it() -> Result<()> {
    for (stdin, want) in [
        (
            "{\"a\":",
            "jq: ignoring parse error: Unfinished abandoned text at EOF at line 1, column 5",
        ),
        (
            "a\nb\n",
            "jq: ignoring parse error: Unfinished abandoned text at EOF at line 3, column 0",
        ),
    ] {
        let (out, err, code) = run_jq_full(&["--seq", "-c", ".", "-"], Some(stdin))?;
        assert_eq!((out.as_str(), code), ("", 0), "{stdin:?}");
        assert_eq!(err.lines().next(), Some(want), "{stdin:?}");
    }
    Ok(())
}
