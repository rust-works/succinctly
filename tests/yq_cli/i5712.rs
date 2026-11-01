//! #5712: a string's `length` composed with a later stage or a `select` condition is answered
//! from the string in yq mode too, and still counts bytes there (`héllo` is 6, not 5). The
//! expectations are live captures from the pinned yq v4.53.3
//! (`yq -n '"héllo" | select(length > 5) | length'` prints `6`, and `select(length > 6)` prints
//! nothing). Only a loop's condition reaches the new sites through a flag real yq has no `until`
//! for, so the per-mode count at those sites is pinned by the unit test
//! `test_owned_string_length_needs_no_second_reindex_5712`, which runs both modes.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn test_composed_string_length_counts_bytes_as_yq_does_5712() -> Result<()> {
    for (filter, want) in [
        ("\"h\u{e9}llo\" | length", "6\n"),
        ("\"h\u{e9}llo\" | length | tostring", "6\n"),
        ("\"h\u{e9}llo\" | select(length > 3) | length", "6\n"),
        // The threshold sits between the codepoint count (5) and the byte count (6).
        ("\"h\u{e9}llo\" | select(length > 5) | length", "6\n"),
        ("\"h\u{e9}llo\" | select(length > 6) | length", ""),
    ] {
        let (stdout, code) = run_yq_stdin(filter, "", &["-n"])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter}");
    }
    Ok(())
}

/// An ordering real yq refuses is still refused when the length is composed into a `select`.
#[test]
fn test_composed_string_length_compared_with_a_string_is_still_refused_5712() -> Result<()> {
    let (stdout, code) = run_yq_stdin("\"abc\" | select(length > \"a\") | length", "", &["-n"])?;
    assert_eq!((stdout.as_str(), code), ("", 1));
    Ok(())
}
