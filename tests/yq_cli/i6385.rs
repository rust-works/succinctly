//! #6385: a `key: value` line indented between a sequence item and its compact mapping is
//! `inconsistent indentation`, as yq v4.53.3 refuses it. The plain indent comparison `close_deeper_indents`
//! applies closed the compact mapping and reopened a second, orphaned mapping as a sibling directly under the same
//! sequence item, which silently dropped every field after the first inconsistent line (#885); reading the line as
//! another entry of the mapping avoided that, and so does refusing it, without a reading yq does not have. Each row
//! is a live yq capture.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

/// Documents yq refuses (`did not find expected '-' indicator`).
const REFUSED: &[&str] = &[
    // The #885 headline repro, later fields, and a properly dashed sibling after it.
    "-   a: hello\n  b: 2\n",
    "-   a: 1\n  b: 2\n  c: 3\n",
    "-   a: 1\n  b: 2\n-   x: 9\n",
    // The most common real-world shape: a single space after the dash, the continuation on the item's own
    // virtual indent.
    "- a: 1\n b: 2\n",
    " - k: a\n  b: 1\n",
    // Every key spelling the dispatch arms handle: `?`/`:`, anchor, tag, alias.
    "-   a: hello\n  ? b\n  : 2\n",
    "-   &anc k: v\n  b: 2\n",
    "-   !!str k: v\n  b: 2\n",
    "x: &k1 hello\nlist:\n-   *k1: v\n  b: 2\n",
    // An own-line value on that indent, and a nested explicit key's continuation.
    "- k:\n x\n",
    "- ? a\n b: 1\n",
    "- ? a\n : 1\n b: 2\n",
];

#[test]
fn test_a_line_between_a_sequence_item_and_its_compact_mapping_is_refused_6385() -> Result<()> {
    for yaml in REFUSED {
        let (output, stderr, exit_code) =
            run_yq_stdin_with_stderr(".", yaml, &["-o", "json", "-I0"])?;
        assert_eq!(exit_code, 1, "{yaml:?}: {output:?}");
        assert_eq!(output, "", "{yaml:?}");
        assert!(
            stderr.contains("inconsistent indentation"),
            "{yaml:?}: {stderr:?}"
        );
    }
    Ok(())
}
