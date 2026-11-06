//! #6295: go-yaml skips a tab only where a simple key is not allowed, which is never at the start of
//! a block-context line, so a line of spaces and a tab and nothing else is `while scanning for the
//! next token` (`while scanning a block scalar`) after every node but the ones whose scanner
//! consumes the blank lines itself: a plain scalar (#6286) and a block scalar's content. `succinctly
//! yq` read it as an empty line. A tab-only line in the head of the first document is yq's header
//! (its slurp takes it away) and reads. Every row was captured from yq v4.53.3
//! (`yq -o=json -I0 .`; `⇥` below is a tab).

use anyhow::Result;

use crate::{run_yq_stdin, run_yq_stdin_with_stderr};

const ARGS: &[&str] = &["-o=json", "-I=0"];

#[test]
fn test_a_tab_only_line_after_a_completed_node_is_refused_as_yq_refuses_it_6295() -> Result<()> {
    for doc in [
        "a: [x]\n \t\n",
        "a: \"x\"\n \t\n",
        "a: 'x'\n  \t\nb: 1\n",
        "a: x # c\n \t\n",
        "a: 1 # c\n \t\n",
        "a:\n \t\n",
        "a:\n \t\nb: 1\n",
        "a:\n \t\n  x\n",
        "a: &x\n \t\n  1\n",
        "a: !!str\n \t\n  x\n",
        "a: &x [1]\n \t\nb: *x\n",
        "- a\n- [x]\n \t\n",
        "- \n \t\n  x\n",
        "- - [x]\n \t\n- y\n",
        "a:\n  - [x]\n   \t\n  - y\n",
        "a: [x]\n \t \nb: 1\n",
        "a: [x]\n\t\nb: 1\n",
        "a: |\n  x\n \t\n",
        "a: 1\n---\n\t\nb: 2\n",
        "a: 1\n---\n \t\nb: 2\n",
        // After a `...` the gap is no header either.
        "---\na: 1\n...\n \t\n---\nb: 2\n",
        "---\na: 1\n...\n\t\n---\nb: 2\n",
        // Before the first content line of a block scalar the indent is not known yet.
        "a: |\n \t\n  x\n  y\nb: 1\n",
        "a: |\n  \t\n  x\n",
        "a: >\n \t \n  x\n",
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(".", doc, ARGS)?;
        assert_ne!(code, 0, "{doc:?} should be refused, got {stdout:?}");
        assert!(stderr.contains("tab"), "{doc:?}: {stderr:?}");
    }
    Ok(())
}

#[test]
fn test_a_tab_only_line_that_yq_reads_still_reads_6295() -> Result<()> {
    for (doc, expected) in [
        // The head of the first document is yq's header.
        ("\t\na: 1\n", "{\"a\":1}"),
        (" \t\na: 1\n", "{\"a\":1}"),
        ("# c\n\t\na: 1\n", "{\"a\":1}"),
        ("---\n\t\na: 1\n", "{\"a\":1}"),
        ("---\n \t\na: 1\n", "{\"a\":1}"),
        ("---\n\t \na: 1\n...\n---\nb: 2\n", "{\"a\":1}\n{\"b\":2}"),
        // A plain scalar consumes the blank lines after it, as before (#6286).
        ("a: x\n \t\n", "{\"a\":\"x\"}"),
        ("x\n\t\n", "\"x\""),
        ("x\n \t\n", "\"x\""),
        ("? a\n \t\n: b\n", "{\"a\":\"b\"}"),
        // A block scalar's content takes it as text.
        ("a: >\n  x\n  \t\n", "{\"a\":\"x\\n\\t\\n\"}"),
        // A flow collection is not block context.
        ("a: [\n \t\nx]\n", "{\"a\":[\"x\"]}"),
        ("a: {\n \t\nb: 1}\n", "{\"a\":{\"b\":1}}"),
        // A tab-only line at the end of the document, after a value.
        ("a: 1\n \t\n", "{\"a\":1}"),
    ] {
        let (stdout, code) = run_yq_stdin(".", doc, ARGS)?;
        assert_eq!((stdout.trim_end(), code), (expected, 0), "{doc:?}");
    }
    Ok(())
}
