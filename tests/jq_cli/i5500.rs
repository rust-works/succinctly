//! #5500: an index write past a sane bound is refused before anything is allocated. jq 1.7.1
//! has no bound (`.[1e12] = 1` kills it with SIGSEGV, exit 139), so there is no output to match;
//! the wording is succinctly's own, as for #1017's `setpath([1e30]; 9)`. Indexes jq 1.7.1 answers
//! still pad: `.[5] = 1` on `[]` is `[null,null,null,null,null,1]` in both.

use anyhow::Result;

use crate::{run_jq_null, run_jq_stdin_streams};

#[test]
fn test_index_write_past_the_bound_is_an_error_not_a_hang_5500() -> Result<()> {
    for filter in [
        "[] | .[1e12] = 1 | length",
        "[] | .[536870912] = 1 | length",
        "null | .[1e12] = 1",
        "[] | setpath([1e12]; 1)",
        "[] | .[1e12] |= 1",
    ] {
        let (stdout, stderr, code) = run_jq_stdin_streams(filter, "null", &["-c"])?;
        assert_eq!(stdout, "", "{filter}");
        assert!(
            stderr.contains("Cannot grow array to"),
            "{filter}: {stderr}"
        );
        assert_eq!(code, 5, "{filter}");
    }
    Ok(())
}

#[test]
fn test_index_write_is_caught_by_try_5500() -> Result<()> {
    let (stdout, code) = run_jq_null("try ([] | .[1e12] = 1) catch \"refused\"", &["-c"])?;
    assert_eq!((stdout.as_str(), code), ("\"refused\"\n", 0));
    Ok(())
}

#[test]
fn test_index_writes_jq_answers_still_pad_5500() -> Result<()> {
    let (stdout, code) = run_jq_null("[] | .[5] = 1", &["-c"])?;
    assert_eq!(
        (stdout.as_str(), code),
        ("[null,null,null,null,null,1]\n", 0)
    );
    Ok(())
}
