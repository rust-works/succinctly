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
