//! #6950: `document_index` of a node a write has produced is the number of the document it was read
//! from, as yq stamps it when it decodes the document: `.x = 5 | document_index` over a stream is
//! `0` then `1`, not `0` twice, so `.x = 5 | select(documentIndex == 1)` finds the second document.
//! The number counts the documents of the file it is in, so a second file starts at `0` again.
//! Navigating to a key the document lacks (`.zz`) is the same node in yq and keeps it too.
//!
//! Every row was captured from yq v4.53.3 (`-o=json -I0`) over [`STREAM`] (one file of two documents)
//! and over [`FILES`] (a file of one, then a file of two), and kept only when both tools agree.
//! A value the filter builds (`[.x]`, a literal) starts a fresh node in yq whose document is `0`;
//! succinctly answers the document being evaluated there, which `docs/compliance/yq/limitations.md`
//! records beside `filename` and `file_index`.

use std::io::Write;

use anyhow::Result;
use tempfile::NamedTempFile;

use crate::{run_yq_files, run_yq_stdin};

const STREAM: &str = "x: 1\ny: 2\n---\nx: 3\ny: 4\n";

const FILES: [&str; 2] = ["a: 1\n", "a: 2\n---\na: 3\n"];

/// (filter, stdout over [`STREAM`], stdout over [`FILES`]).
const ROWS: &[(&str, &str, &str)] = &[
    (".x = 5 | documentIndex", "0\n1\n", "0\n0\n1\n"),
    (".x |= . + 1 | di", "0\n1\n", "0\n0\n1\n"),
    ("del(.x) | document_index", "0\n1\n", "0\n0\n1\n"),
    (". * {\"z\":1} | document_index", "0\n1\n", "0\n0\n1\n"),
    (". + {\"z\":1} | di", "0\n1\n", "0\n0\n1\n"),
    ("with(.x; . = 5) | document_index", "0\n1\n", "0\n0\n1\n"),
    (
        ".x = 5 | select(documentIndex == 1)",
        "{\"x\":5,\"y\":4}\n",
        "{\"a\":3,\"x\":5}\n",
    ),
    (
        ".a = 9 | select(di == 1)",
        "{\"x\":3,\"y\":4,\"a\":9}\n",
        "{\"a\":9}\n",
    ),
    (".x | document_index", "0\n1\n", "0\n0\n1\n"),
    (".zz | document_index", "0\n1\n", "0\n0\n1\n"),
    (".x = 1 | .y | di", "0\n1\n", "0\n0\n1\n"),
    (
        "select(di == 1) | .x = 7",
        "{\"x\":7,\"y\":4}\n",
        "{\"a\":3,\"x\":7}\n",
    ),
    (".x = 5 | .. | select(. == 5) | di", "0\n1\n", "0\n0\n1\n"),
    ("(.x, .y) = 9 | di", "0\n1\n", "0\n0\n1\n"),
];

#[test]
fn a_node_a_write_produced_keeps_its_document_number_6950() -> Result<()> {
    for (filter, stream, _) in ROWS {
        for extra in [&["-o=json", "-I0"][..], &["-P", "-o=json", "-I0"]] {
            let (stdout, code) = run_yq_stdin(filter, STREAM, extra)?;
            assert_eq!(code, 0, "{filter} {extra:?}");
            assert_eq!(&stdout, stream, "{filter} {extra:?}");
        }
    }
    Ok(())
}

#[test]
fn a_node_a_write_produced_keeps_its_document_number_over_several_files_6950() -> Result<()> {
    let mut files = Vec::new();
    for text in FILES {
        let mut file = NamedTempFile::new()?;
        write!(file, "{text}")?;
        files.push(file);
    }
    let paths: Vec<_> = files.iter().map(NamedTempFile::path).collect();
    for (filter, _, expected) in ROWS {
        let (stdout, stderr, code) = run_yq_files(filter, &paths, &["-o=json", "-I0"])?;
        assert_eq!(code, 0, "{filter}: {stderr}");
        assert_eq!(&stdout, expected, "{filter}");
    }
    Ok(())
}

/// A JSON stream of several values is several documents too (#2839).
#[test]
fn a_json_stream_keeps_its_document_numbers_through_a_write_6950() -> Result<()> {
    let (stdout, code) = run_yq_stdin(
        ".x = 5 | document_index",
        "{\"x\":1}\n{\"x\":3}\n",
        &["-p=json", "-o=json", "-I0"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "0\n1\n");
    Ok(())
}

/// `--doc N` selects by the stream's global number; the node it reads keeps the number it has in
/// its file, which is the same here.
#[test]
fn doc_selects_by_the_global_number_and_a_write_keeps_it_6950() -> Result<()> {
    let (stdout, code) = run_yq_stdin(".x = 5 | document_index", STREAM, &["--doc", "1"])?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "1\n");
    Ok(())
}

/// `-i` rewrites each document in place, and a filter after the write still finds the second one.
#[test]
fn inplace_keeps_the_document_number_through_a_write_6950() -> Result<()> {
    let mut file = NamedTempFile::new()?;
    write!(file, "x: 1\n---\nx: 2\n")?;
    let (_, stderr, code) = run_yq_files(
        ".x = 9 | select(document_index == 1) | .z = 3",
        &[file.path()],
        &["-i"],
    )?;
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(std::fs::read_to_string(file.path())?, "x: 9\nz: 3\n");
    Ok(())
}

/// Where succinctly answers differently from yq (`0`), recorded in
/// `docs/compliance/yq/limitations.md`: a node the filter builds, and a `load()`ed one, answer
/// the document being evaluated.
#[test]
fn a_built_node_answers_the_document_being_evaluated_6950() -> Result<()> {
    for filter in ["[.x] | document_index", "\"s\" | document_index"] {
        let (stdout, code) = run_yq_stdin(filter, STREAM, &["-o=json", "-I0"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(stdout, "0\n1\n", "{filter}");
    }
    Ok(())
}
