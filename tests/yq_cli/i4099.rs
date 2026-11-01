//! #4099: an input with no bytes at all is one empty document in yq, not an error: `printf '' |
//! yq '.a'` is `null`, `.a = 1` is `a: 1`, `length` is `0`. `succinctly yq` stopped with
//! `YAML parse error: empty input` (exit 1). Each row is a live yq v4.53.3 capture over empty
//! stdin; an empty file, and `-` for stdin, answer the same.
//!
//! yq reads no document from an empty file and evaluates once against an empty one when a run read none;
//! the rows with several files pin that.

use anyhow::Result;
use tempfile::NamedTempFile;

use crate::{run_yq_files, run_yq_stdin};

/// (filter, expected stdout).
const ROWS: &[(&str, &str)] = &[
    (".", "\n"),
    (".a", "null\n"),
    (".a = 1", "a: 1\n"),
    ("length", "0\n"),
    ("type", "!!null\n"),
    (". == null", "true\n"),
    ("document_index", "0\n"),
    ("tag", "!!null\n"),
    ("(.a, .b)", "null\nnull\n"),
];

#[test]
fn test_empty_stdin_is_one_empty_document_4099() -> Result<()> {
    for (filter, expected) in ROWS {
        let (stdout, code) = run_yq_stdin(filter, "", &[])?;
        assert_eq!((stdout.as_str(), code), (*expected, 0), "{filter}");
    }
    let (stdout, code) = run_yq_stdin(".a", "", &["-o", "json", "-I", "0"])?;
    assert_eq!((stdout.as_str(), code), ("null\n", 0));
    let (stdout, code) = run_yq_stdin(".a", "", &["-p", "json"])?;
    assert_eq!((stdout.as_str(), code), ("null\n", 0));
    Ok(())
}

#[test]
fn test_empty_file_is_one_empty_document_4099() -> Result<()> {
    let empty = NamedTempFile::new()?;
    for (filter, expected) in ROWS {
        let (stdout, stderr, code) = run_yq_files(filter, &[empty.path()], &[])?;
        assert_eq!(
            (stdout.as_str(), code),
            (*expected, 0),
            "{filter}: {stderr}"
        );
    }
    Ok(())
}

/// An empty file among several contributes nothing; every file empty is one empty document.
#[test]
fn test_empty_file_among_several_inputs_4099() -> Result<()> {
    let empty = NamedTempFile::new()?;
    let other_empty = NamedTempFile::new()?;
    let mut one = NamedTempFile::new()?;
    std::io::Write::write_all(&mut one, b"a: 1\n")?;
    for (filter, files, expected) in [
        (".a", vec![empty.path(), one.path()], "1\n"),
        (".a", vec![one.path(), empty.path()], "1\n"),
        (
            ".a",
            vec![one.path(), empty.path(), one.path()],
            "1\n---\n1\n",
        ),
        (".a", vec![empty.path(), other_empty.path()], "null\n"),
        (".a = 1", vec![empty.path(), other_empty.path()], "a: 1\n"),
        ("file_index", vec![empty.path(), other_empty.path()], "0\n"),
        ("file_index", vec![empty.path(), one.path()], "1\n"),
    ] {
        let (stdout, stderr, code) = run_yq_files(filter, &files, &[])?;
        assert_eq!((stdout.as_str(), code), (expected, 0), "{filter}: {stderr}");
    }
    Ok(())
}

/// An empty `.json` input answers `null` wherever the output is JSON (#5478 pins the blank line it
/// is where the output is YAML).
#[test]
fn test_empty_json_input_is_null_4099() -> Result<()> {
    let dir = tempfile::TempDir::new()?;
    let path = dir.path().join("e.json");
    std::fs::write(&path, "")?;
    for (filter, args, expected) in [
        (".", vec!["-o", "json"], "null\n"),
        (".a", vec![], "null\n"),
        (".a = 1", vec!["-o", "json", "-I", "0"], "{\"a\":1}\n"),
    ] {
        let (stdout, stderr, code) = run_yq_files(filter, &[path.as_path()], &args)?;
        assert_eq!((stdout.as_str(), code), (expected, 0), "{filter}: {stderr}");
    }
    let (stdout, code) = run_yq_stdin(".", "", &["-p", "json", "-o", "json"])?;
    assert_eq!((stdout.as_str(), code), ("null\n", 0));
    Ok(())
}
