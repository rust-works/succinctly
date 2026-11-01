//! #5712: a string's `length` composed with a later stage or a `select` condition is answered
//! from the string in yq mode too, and still counts bytes there (`h\u{e9}llo` is 6). The
//! expectations were captured from a live yq (`yq -n '"h\u{e9}llo" | length'` prints `6`); the
//! machine that ran them had v4.49.2, not the pinned v4.53.3, and the answer is the same
//! byte count as the one `succinctly yq` gave before the change.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn test_composed_string_length_counts_bytes_as_yq_does_5712() -> Result<()> {
    for filter in [
        "\"h\u{e9}llo\" | length",
        "\"h\u{e9}llo\" | length | tostring",
        "\"h\u{e9}llo\" | select(length > 3) | length",
    ] {
        let (stdout, code) = run_yq_stdin(filter, "", &["-n"])?;
        assert_eq!((stdout.as_str(), code), ("6\n", 0), "{filter}");
    }
    Ok(())
}
