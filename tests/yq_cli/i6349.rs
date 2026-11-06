//! #6349: yq's header slurp sets aside every leading blank, comment, directive and `---` line before
//! the first content line, so a run of leading bare markers is one document: `---` / `---` / `x` is
//! `"x"`, not a null document and then `x`. A marker after content is still a marker. Every row was
//! captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

const JSON: &[&str] = &["-o=json", "-I=0"];

#[test]
fn test_a_run_of_leading_bare_markers_is_one_document_6349() -> Result<()> {
    for (doc, expected) in [
        ("---\n---\nx\n", "\"x\"\n"),
        ("---\n---\n", "null\n"),
        ("---\n---\n---\n", "null\n"),
        ("---\n---\n---\nx\n", "\"x\"\n"),
        ("---\n---\na: 1\n", "{\"a\":1}\n"),
        ("---\n---\nx: 1\ny: 2\n", "{\"x\":1,\"y\":2}\n"),
        ("---\n---\n- a\n- b\n", "[\"a\",\"b\"]\n"),
        ("---\n---\n# t\n", "null\n"),
        ("# c\n---\n---\nx\n", "\"x\"\n"),
        ("---\n# c\n---\nx\n", "\"x\"\n"),
        ("---\n\n---\nx\n", "\"x\"\n"),
        ("\n---\n---\nx\n", "\"x\"\n"),
        ("%YAML 1.2\n---\n---\nx\n", "\"x\"\n"),
        ("---\n--- x\ny\n", "\"xy\"\n"),
        // The run is the first document only; what follows it is as before.
        ("---\n---\nx\n---\ny\n", "\"x\"\n\"y\"\n"),
        ("---\n---\nx\n...\n---\ny\n", "\"x\"\n\"y\"\n"),
    ] {
        let (stdout, code) = run_yq_stdin(".", doc, JSON)?;
        assert_eq!((stdout.as_str(), code), (expected, 0), "{doc:?}");
    }
    Ok(())
}

#[test]
fn test_a_marker_after_content_or_an_empty_later_document_is_still_a_document_6349() -> Result<()> {
    for (doc, expected) in [
        ("a: 1\n---\n---\nb: 2\n", "{\"a\":1}\nnull\n{\"b\":2}\n"),
        ("x\n---\n---\ny\n", "\"x\"\nnull\n\"y\"\n"),
        ("---\nx\n---\n---\ny\n", "\"x\"\nnull\n\"y\"\n"),
    ] {
        let (stdout, code) = run_yq_stdin(".", doc, JSON)?;
        assert_eq!((stdout.as_str(), code), (expected, 0), "{doc:?}");
    }
    Ok(())
}

#[test]
fn test_the_whole_header_of_a_marker_run_is_printed_once_6349() -> Result<()> {
    for (doc, expected) in [
        ("---\n---\nx\n", "---\n---\nx\n"),
        ("---\n---\na: 1\n", "---\n---\na: 1\n"),
        ("---\n---\n---\nx\n", "---\n---\n---\nx\n"),
        ("# c\n---\n---\nx\n", "# c\n---\n---\nx\n"),
        ("---\n# c\n---\nx\n", "---\n# c\n---\nx\n"),
        ("---\n\n---\nx\n", "---\n\n---\nx\n"),
        ("%YAML 1.2\n---\n---\nx\n", "%YAML 1.2\n---\n---\nx\n"),
        ("---\n---\n# t\n", "---\n---\n# t\n"),
        ("---\n---\n# c\nb: 2\n", "---\n---\n# c\nb: 2\n"),
        ("---\n---\nx\n---\ny\n", "---\n---\nx\n---\ny\n"),
    ] {
        let (stdout, code) = run_yq_stdin(".", doc, &[])?;
        assert_eq!((stdout.as_str(), code), (expected, 0), "{doc:?}");
    }
    Ok(())
}

#[test]
fn test_the_run_is_document_zero_and_numbers_its_lines_from_the_content_6349() -> Result<()> {
    for (doc, filter, expected) in [
        ("---\n---\nx\n---\ny\n", "document_index", "0\n1\n"),
        ("---\n---\na: 1\n", "document_index", "0\n"),
        ("---\n---\na: 1\n", "line", "1\n"),
        ("---\n# c\n---\nk: 1\nm: 2\n", ".m | line", "2\n"),
        ("---\n---\nk: 1\n", ".k | line", "1\n"),
    ] {
        let (stdout, code) = run_yq_stdin(filter, doc, &[])?;
        let separated = stdout.replace("---\n", "");
        assert_eq!(
            (separated.as_str(), code),
            (expected, 0),
            "{doc:?} {filter}"
        );
    }
    Ok(())
}

#[test]
fn test_the_run_is_one_document_for_eval_all_and_in_place_editing_6349() -> Result<()> {
    let (stdout, code) = run_yq_stdin("[.]", "---\n---\nx\n", &["--eval-all", "-o=json", "-I=0"])?;
    assert_eq!((stdout.as_str(), code), ("[\"x\"]\n", 0));
    let (stdout, code) = run_yq_stdin(".a = 1", "---\n---\nb: 2\n", &["-o=json", "-I=0"])?;
    assert_eq!((stdout.as_str(), code), ("{\"b\":2,\"a\":1}\n", 0));
    Ok(())
}
