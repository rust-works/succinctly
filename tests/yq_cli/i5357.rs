//! #5357: a UTF-8 byte-order mark in front of a YAML stream leaves yq's header slurp nothing to set
//! aside (`--header-preprocess` looks at the first byte, and a mark is not a blank, a comment, a
//! directive or a `---`), so yq decodes the whole text as an ordinary stream: the first document's
//! explicit `---` is read, not echoed, an empty first document prints as a blank line, and blank
//! lines and a comment's indent ahead of the first content are normalised away. Every row is a live
//! yq v4.53.3 capture (`printf '\xef\xbb\xbf<text>' | yq <flags> .`).
//!
//! The streaming routes read it (stdin and files, `.`, `-N`, `-I=0`, `-o=json`, a path). The DOM
//! routes (`-P`, a write, `--eval-all`, `--slurp`) still model the header and keep printing the
//! leading `---` (`test_the_dom_route_still_echoes_the_marker_after_a_bom_5357`).

use std::io::Write;

use anyhow::Result;
use tempfile::NamedTempFile;

use crate::{run_yq_files, run_yq_stdin};

const BOM: &str = "\u{feff}";

/// `(text after the mark, yq's output for `.`)`.
const DEFAULT_OUTPUT: &[(&str, &str)] = &[
    // The leading `---` is not echoed; a marker between documents is.
    ("---\na: 1\n---\nb: 2\n", "a: 1\n---\nb: 2\n"),
    ("---\n- 1\n- 2\n", "- 1\n- 2\n"),
    // A leading run of markers is a null document and then the next one.
    ("---\n---\na: 1\n", "\n---\na: 1\n"),
    ("---\n...\n---\na: 1\n", "\n---\na: 1\n"),
    // What lies in front of the first content: a comment stays, blank lines and a comment's
    // indent do not.
    ("# c\n---\na: 1\n", "# c\na: 1\n"),
    ("\n---\na: 1\n", "a: 1\n"),
    ("---\n\na: 1\n", "a: 1\n"),
    ("# c\n\na: 1\n", "# c\na: 1\n"),
    ("  # c\na: 1\n", "# c\na: 1\n"),
    // No content at all is the one empty document, a blank line, and its comment is lost.
    ("---\n", "\n"),
    ("# only\n", "\n"),
    ("\n\n", "\n"),
    ("  \n", "\n"),
];

#[test]
fn test_a_bom_leaves_no_header_so_a_leading_marker_is_read_on_stdin_5357() -> Result<()> {
    for (text, want) in DEFAULT_OUTPUT {
        let input = format!("{BOM}{text}");
        let (stdout, code) = run_yq_stdin(".", &input, &[])?;
        assert_eq!((stdout.as_str(), code), (*want, 0), "{text:?}");
    }
    Ok(())
}

#[test]
fn test_a_bom_leaves_no_header_so_a_leading_marker_is_read_in_a_file_5357() -> Result<()> {
    for (text, want) in DEFAULT_OUTPUT {
        let mut file = NamedTempFile::new()?;
        write!(file, "{BOM}{text}")?;
        let (stdout, stderr, code) = run_yq_files(".", &[file.path()], &[])?;
        assert_eq!((stdout.as_str(), code), (*want, 0), "{text:?}: {stderr}");
    }
    Ok(())
}

#[test]
fn test_the_marker_stays_without_a_bom_5357() -> Result<()> {
    // The control: the same text with no mark is all header, and its `---` is echoed.
    for (text, want) in [
        ("---\na: 1\n---\nb: 2\n", "---\na: 1\n---\nb: 2\n"),
        ("---\n---\na: 1\n", "---\n---\na: 1\n"),
        ("# c\n\na: 1\n", "# c\n\na: 1\n"),
    ] {
        let (stdout, code) = run_yq_stdin(".", text, &[])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{text:?}");
    }
    Ok(())
}

#[test]
fn test_flags_and_paths_after_a_bom_5357() -> Result<()> {
    let text = format!("{BOM}---\na: 1\n---\nb: 2\n");
    for (extra, filter, want) in [
        // `-N` drops the separator, as it does for any stream.
        (&["-N"][..], ".", "a: 1\nb: 2\n"),
        (&["-I=0"][..], ".", "a: 1\n---\nb: 2\n"),
        (&["-o=json", "-I=0"][..], ".", "{\"a\":1}\n{\"b\":2}\n"),
        (&[][..], ".a", "1\n---\nnull\n"),
    ] {
        let (stdout, code) = run_yq_stdin(filter, &text, extra)?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{extra:?} {filter}");
    }
    Ok(())
}

#[test]
fn test_an_input_other_than_the_first_keeps_its_own_header_model_5357() -> Result<()> {
    // Each file is a stream of its own: the one with a mark has no header, the one without does.
    let mut plain = NamedTempFile::new()?;
    write!(plain, "---\na: 1\n")?;
    let mut bom = NamedTempFile::new()?;
    write!(bom, "{BOM}---\nb: 2\n")?;
    let (stdout, stderr, code) = run_yq_files(".", &[plain.path(), bom.path()], &[])?;
    assert_eq!(
        (stdout.as_str(), code),
        ("---\na: 1\n---\nb: 2\n", 0),
        "{stderr}"
    );
    let (stdout, stderr, code) = run_yq_files(".", &[bom.path(), bom.path()], &[])?;
    assert_eq!(
        (stdout.as_str(), code),
        ("b: 2\n---\nb: 2\n", 0),
        "{stderr}"
    );
    Ok(())
}

#[test]
fn test_the_dom_route_still_echoes_the_marker_after_a_bom_5357() -> Result<()> {
    // Not yq's output (`a: 1`): the DOM routes keep the header model, so this is a recorded
    // difference (docs/compliance/yq/limitations.md), pinned so the page stays true.
    let input = format!("{BOM}---\na: 1\n");
    let (stdout, code) = run_yq_stdin(".", &input, &["-P"])?;
    assert_eq!((stdout.as_str(), code), ("---\na: 1\n", 0));
    Ok(())
}
