//! #5089: a document after a `...` end marker needs its own `---`. Content that follows `...` with no
//! `---` read as a further document (`a` / `...` / `b` was `"a"` then `"b"`); yq prints the first and
//! stops with `did not find expected <document start>`, and so does `succinctly yq` now (exit 1, with
//! no output: yq has printed the first document before the error, which this does not). Each row is a
//! live yq v4.53.3 capture. A directive after `...` stays accepted (YAML 1.2 allows it, yq rejects it).

use anyhow::Result;

use crate::{run_yq_stdin, run_yq_stdin_with_stderr};

const FIRSTS: &[&str] = &["a", "\"q\"", "[1]", "a: 1", "- a", "|\n  t"];
const AFTERS: &[&str] = &[
    "b",
    "b: 1",
    "- b",
    "[x]",
    "\"r\"",
    "# c\nb",
    "\nb",
    "...\nb",
    "... # c\nb",
];

#[test]
fn test_content_after_a_document_end_marker_without_a_start_is_rejected_5089() -> Result<()> {
    for first in FIRSTS {
        for after in AFTERS {
            let doc = format!("{first}\n...\n{after}\n");
            let (stdout, stderr, code) = run_yq_stdin_with_stderr(".", &doc, &[])?;
            assert_eq!((stdout.as_str(), code), ("", 1), "{doc:?}");
            assert!(
                stderr.contains("did not find expected <document start>"),
                "{doc:?}: {stderr}"
            );
        }
    }
    Ok(())
}

/// Other spellings yq rejects: content on the marker's own line, CRLF, and a directive followed by
/// content with no `---`. A marker that opens the stream is refused for another reason, and is
/// pinned in `i6342.rs`.
const REJECTED_OTHER: &[&str] = &["a\n... b\n", "a\r\n...\r\nb\r\n", "a\n...\n%YAML 1.1\nb\n"];

#[test]
fn test_other_spellings_of_content_after_an_end_marker_are_rejected_5089() -> Result<()> {
    for doc in REJECTED_OTHER {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(".", doc, &[])?;
        assert_eq!((stdout.as_str(), code), ("", 1), "{doc:?}");
        assert!(
            stderr.contains("did not find expected <document start>"),
            "{doc:?}: {stderr}"
        );
    }
    Ok(())
}

/// Accepted by yq: (document, `-o json -I0` output).
const ACCEPTED: &[(&str, &str)] = &[
    ("a\n...\n---\nb\n", "\"a\"\n\"b\"\n"),
    ("a\n...\n# c\n---\nb\n", "\"a\"\n\"b\"\n"),
    ("a\n...\n\n\n---\nb\n", "\"a\"\n\"b\"\n"),
    ("\"q\"\n...\n--- |\n  t\n", "\"q\"\n\"t\\n\"\n"),
    ("a: 1\n...\n--- b\n", "{\"a\":1}\n\"b\"\n"),
    ("a\n...\n", "\"a\"\n"),
    ("a\n... # c\n", "\"a\"\n"),
    ("a\n...\n# only a comment\n", "\"a\"\n"),
    ("a\n...\n...\n", "\"a\"\n"),
    ("a\n---\nb\n...\n---\nc\n", "\"a\"\n\"b\"\n\"c\"\n"),
    // A `%YAML 1.1` after `...` is go-yaml's to read, and it reads it (#6419).
    ("a\n...\n%YAML 1.1\n---\nb\n", "\"a\"\n\"b\"\n"),
];

#[test]
fn test_documents_separated_by_a_start_marker_after_an_end_marker_are_unchanged_5089() -> Result<()>
{
    for (doc, expected) in ACCEPTED {
        let (stdout, code) = run_yq_stdin(".", doc, &["-o", "json", "-I0"])?;
        assert_eq!((stdout.as_str(), code), (*expected, 0), "{doc:?}");
    }
    Ok(())
}

/// A stream whose first document is `--- text` followed by `...` and more content: yq folds the
/// `...` into the scalar (`"v... b"`), because its header slurp glues the line after the marker
/// line onto it, so `...` is no longer at the start of a line (#6276). This was a recorded
/// divergence of #5089 (an error here, two documents before it); it reads as yq does now.
const FOLDED_BY_YQ: &[(&str, &str)] = &[
    ("--- v\n...\nb\n", "\"v... b\"\n"),
    ("--- v w\n...\n  b\n", "\"v w... b\"\n"),
    ("--- &a v\n...\nb\n", "\"v... b\"\n"),
];

#[test]
fn test_yq_folds_the_end_marker_into_an_inline_scalar_5089() -> Result<()> {
    for (doc, expected) in FOLDED_BY_YQ {
        let (stdout, code) = run_yq_stdin(".", doc, &["-o", "json", "-I0"])?;
        assert_eq!((stdout.as_str(), code), (*expected, 0), "{doc:?}");
    }
    Ok(())
}
