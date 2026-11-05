//! #6231: a literal-bound slice of an array is read where it stands in yq mode too, and answers
//! as before (captured from yq v4.53.3): the slice is a new block-style sequence, whatever
//! style the source had.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn test_literal_array_slice_keeps_yq_answers_6231() -> Result<()> {
    for (filter, input, expected) in [
        (".c[0:2]", "c: [1, 2, 3]\n", "- 1\n- 2\n"),
        (".c[1:]", "c: [1, 2, 3]\n", "- 2\n- 3\n"),
        (".c[:-1]", "c:\n  - 1\n  - 2\n  - 3\n", "- 1\n- 2\n"),
        (".c[5:9]", "c: [1, 2, 3]\n", "[]\n"),
    ] {
        let (stdout, code) = run_yq_stdin(filter, input, &[])?;
        assert_eq!((stdout.as_str(), code), (expected, 0), "{filter}");
    }
    Ok(())
}

/// The sequence `- a`, `- "bad\x"`, `- 3`, `- 4` has an invalid escape in its second element: yq
/// rejects the document when it reads it. A slice that drops the element no longer reads it (the
/// fault is inside a leaf, which no part of the slice decodes); a slice that keeps it still raises.
/// Recorded on the yq limitations page (#6231).
#[test]
fn test_array_slice_decodes_only_what_it_keeps_in_yq_mode_6231() -> Result<()> {
    let input = "- a\n- \"bad\\x\"\n- 3\n- 4\n";
    for (filter, expected) in [(".[:1]", "- a\n"), (".[2:]", "- 3\n- 4\n")] {
        let (stdout, code) = run_yq_stdin(filter, input, &[])?;
        assert_eq!((stdout.as_str(), code), (expected, 0), "{filter}");
    }
    for filter in [".[1:2]", ".[:2]", ".[1:]"] {
        let (stdout, code) = run_yq_stdin(filter, input, &[])?;
        assert_eq!((stdout.as_str(), code), ("", 1), "{filter}");
    }
    Ok(())
}
