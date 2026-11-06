//! #6342: a stream that holds no content is one null document to yq, or an error, depending on what
//! its head is made of. yq's header slurp sets aside every leading blank, comment, `%YAML` directive
//! and `---` line before go-yaml sees the rest, so:
//!
//! - a stream of only those lines is one null document, whose comments and `%YAML` lines print back
//!   as they were (`⇥ # c`, `%YAML 1.2`);
//! - a `...` met while that head is all there is is the first token go-yaml reads, as an implicit
//!   document with no node to close, and is refused;
//! - any other directive (`%TAG`) stays with go-yaml, which wants a `---` after it.
//!
//! A `---` that is not in the head (after content) is read by go-yaml, so `a: 1` / `---` / `...` is
//! an empty second document. Every row was captured from yq v4.53.3.

use anyhow::Result;

use crate::{run_yq_stdin, run_yq_stdin_with_stderr};

const JSON: &[&str] = &["-o=json", "-I=0"];

#[test]
fn a_stream_of_only_a_tab_led_comment_or_a_yaml_directive_is_one_null_document_6342() -> Result<()>
{
    for doc in [
        "\t\n",
        "\t",
        "\t\n\t\n",
        "\t # c\n",
        "\t# c\n",
        " \t# c\n",
        "\t # c",
        "%YAML 1.2\n",
        "%YAML 1.2\n\n",
        "%YAML 1.2\n# c\n",
        "%YAML 1.2",
        "%YAML 1.2\n%YAML 1.2\n",
        "# c\n%YAML 1.2\n",
        "\n%YAML 1.2\n",
        "%YAML 1.2 # c\n",
        "%YAML1.2\n",
        "%YAML 1.2\n\t# c\n",
        "---\n\t# c\n",
        "---\n\t\n",
        // `--- # c` / `...`: the comment swallows the marker line below it (#6276).
        "--- # c\n...\n",
        "%TAG ! tag:x,2000:\n---\n...\n",
        // A `%YAML` line after the leading `---` is header too.
        "---\n%YAML 1.2\n",
        "--- \n%YAML 1.2\n",
        "---\n%YAML 1.2\n---\n",
        "\t\n---\n%YAML 1.2\n",
        "---\n# c\n%YAML 1.2\n\t# d\n",
    ] {
        let (stdout, code) = run_yq_stdin(".", doc, JSON)?;
        assert_eq!((stdout.as_str(), code), ("null\n", 0), "{doc:?}");
    }
    Ok(())
}

#[test]
fn a_tab_led_comment_or_a_yaml_directive_stream_prints_back_as_written_6342() -> Result<()> {
    for doc in [
        "\t # c\n",
        "\t# c\n",
        "%YAML 1.2\n",
        "%YAML 1.2\n\n",
        "%YAML 1.2\n# c\n",
        "%YAML 1.2\n%YAML 1.2\n",
        "# c\n%YAML 1.2\n",
        "\n%YAML 1.2\n",
        "%YAML 1.2 # c\n",
        "%YAML 1.2\n\t# c\n",
        "---\n\t# c\n",
        "---\n%YAML 1.2\n",
        "# c\n---\n%YAML 1.2\n",
        "---\n%YAML 1.2\na: 1\n",
        "---\n# c\n%YAML 1.2\n- x\n",
    ] {
        let (stdout, code) = run_yq_stdin(".", doc, &[])?;
        assert_eq!((stdout.as_str(), code), (doc, 0), "{doc:?}");
    }
    // The marker line is printed bare, whatever blanks followed it.
    let (stdout, code) = run_yq_stdin(".", "--- \n%YAML 1.2\na: 1\n", &[])?;
    assert_eq!((stdout.as_str(), code), ("---\n%YAML 1.2\na: 1\n", 0));
    Ok(())
}

#[test]
fn an_end_marker_met_while_the_head_is_all_there_is_is_refused_6342() -> Result<()> {
    for doc in [
        "...\n",
        "...\n...\n",
        "# c\n...\n",
        "\n...\n",
        "---\n...\n",
        "--- \n...\n",
        "---\n# c\n...\n",
        "---\n---\n...\n",
        "# c\n---\n...\n",
        "%YAML 1.2\n...\n",
        "%YAML 1.2\n---\n...\n",
        "---\n%YAML 1.2\n...\n",
        "...\n---\na: 1\n",
        "...\na: 1\n",
        "...\nb\n",
        "# c\n...\nb\n",
    ] {
        let (out, err, code) = run_yq_stdin_with_stderr(".", doc, JSON)?;
        assert_eq!(out, "", "{doc:?}");
        assert_ne!(code, 0, "{doc:?}");
        // CI appends a backtrace after the first line, so only that line is read.
        let first = err.lines().next().unwrap_or_default();
        assert!(
            first.contains("did not find expected node content"),
            "{doc:?}: {err}"
        );
    }
    Ok(())
}

#[test]
fn a_directive_other_than_yaml_with_no_document_start_after_it_is_refused_6342() -> Result<()> {
    for doc in [
        "%TAG ! tag:x,2000:\n",
        "%YAML 1.2\n%TAG ! x:\n",
        "%TAG ! x:\n%YAML 1.1\n",
        // The directive is go-yaml's to read, so what follows it has to be a `---`.
        "%TAG ! x:\n...\n",
        "%TAG ! x:\na: 1\n",
        "%TAG ! x:\n- a\n",
    ] {
        let (out, err, code) = run_yq_stdin_with_stderr(".", doc, JSON)?;
        assert_eq!(out, "", "{doc:?}");
        assert_ne!(code, 0, "{doc:?}");
        let first = err.lines().next().unwrap_or_default();
        assert!(
            first.contains("did not find expected <document start>"),
            "{doc:?}: {err}"
        );
    }
    Ok(())
}

#[test]
fn an_end_marker_after_content_or_after_a_marker_that_is_not_in_the_head_is_read_as_before_6342(
) -> Result<()> {
    for (doc, expected) in [
        // That `---` is go-yaml's, not the slurp's: an empty document, closed by the `...`.
        ("a: 1\n---\n...\n", "{\"a\":1}\nnull\n"),
        ("a: 1\n...\n---\n...\n", "{\"a\":1}\nnull\n"),
        ("a: 1\n...\n...\n", "{\"a\":1}\n"),
        ("a: 1\n...\n", "{\"a\":1}\n"),
        ("a: 1\n...\n\n...\n---\nb: 2\n", "{\"a\":1}\n{\"b\":2}\n"),
        // A property is a node, an empty scalar, though nothing is written for it: not an empty head.
        ("---\n!!str\n...\n", "\"\"\n"),
        ("!!str\n...\n", "\"\"\n"),
        ("# c\n!!str\n...\n", "\"\"\n"),
        ("--- !!str\n...\n", "\"\"\n"),
        ("---\n&a\n...\n", "null\n"),
        ("&a\n...\n", "null\n"),
        // A head followed by content is not a head-only stream.
        ("\t\na: 1\n", "{\"a\":1}\n"),
        ("\t # c\na: 1\n", "{\"a\":1}\n"),
        ("%YAML 1.2\na: 1\n", "{\"a\":1}\n"),
        ("%YAML 1.2\n---\na: 1\n", "{\"a\":1}\n"),
        ("---\n%YAML 1.2\n---\nx\n", "\"x\"\n"),
        ("---\n%YAML 1.2\na: 1\n", "{\"a\":1}\n"),
        ("%TAG !e! tag:x,2000:\n---\na: 1\n", "{\"a\":1}\n"),
    ] {
        let (stdout, code) = run_yq_stdin(".", doc, JSON)?;
        assert_eq!((stdout.as_str(), code), (expected, 0), "{doc:?}");
    }
    Ok(())
}

#[test]
fn a_tab_led_comment_after_content_is_still_refused_6342() -> Result<()> {
    // A property is content to go-yaml, so the tab-led line after it is not header.
    for doc in [
        "a: 1\n\t# c\n",
        "a:\n\t# c\n  b: 1\n",
        "!!str\n\t# c\n",
        "&a\n\t# c\n",
        "- x\n\t# c\n",
        "%TAG ! x:\n---\n\t# c\n",
    ] {
        let (out, _err, code) = run_yq_stdin_with_stderr(".", doc, JSON)?;
        assert_eq!(out, "", "{doc:?}");
        assert_ne!(code, 0, "{doc:?}");
    }
    Ok(())
}
