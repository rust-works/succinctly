//! #6392: a single-quoted key holding an escaped quote (`''`) and ` # ` is not a comment start, so a compact mapping
//! whose key looks like that and whose value is a block sequence on the next line is read as yq reads it. The text
//! scan that decides whether the previous line's key still waits for its value took the first `'` of the `''`
//! pair for the closing quote and the ` #` after it for a comment, so the key looked complete and the sequence
//! "followed a completed mapping entry". Each row is a live yq v4.53.3 capture.

use crate::run_yq_stdin;

/// (document, `-o json -I0 .` output).
const ROWS: &[(&str, &str)] = &[
    (
        "- 'a ''b'' # c':\n  - cell\n",
        "[{\"a 'b' # c\":[\"cell\"]}]\n",
    ),
    (
        "- 'row \"7\" - ''v'' # c (e.g. 1)':\n  - cell\n  - cell2\n",
        "[{\"row \\\"7\\\" - 'v' # c (e.g. 1)\":[\"cell\",\"cell2\"]}]\n",
    ),
    ("- 'a ''b''':\n  - cell\n", "[{\"a 'b'\":[\"cell\"]}]\n"),
    ("- 'a # b':\n  - cell\n", "[{\"a # b\":[\"cell\"]}]\n"),
    ("- \"a # b\":\n  - cell\n", "[{\"a # b\":[\"cell\"]}]\n"),
    ("- 'a ''b'' # c': v\n", "[{\"a 'b' # c\":\"v\"}]\n"),
    // A key with an anchor after the colon, and a nested level.
    (
        "- 'a '' # b': &x\n  - cell\n",
        "[{\"a ' # b\":[\"cell\"]}]\n",
    ),
    (
        "k:\n  - 'a ''b'' # c':\n    - cell\n",
        "{\"k\":[{\"a 'b' # c\":[\"cell\"]}]}\n",
    ),
    // Unchanged: an apostrophe inside a plain scalar and a real trailing comment after a quoted key.
    ("- it's # c\n- x\n", "[\"it's\",\"x\"]\n"),
    ("- 'a ''b''': # c\n  - cell\n", "[{\"a 'b'\":[\"cell\"]}]\n"),
];

#[test]
fn test_a_doubled_quote_in_a_single_quoted_key_does_not_end_it_6392() {
    for (doc, expected) in ROWS {
        let (stdout, code) = run_yq_stdin(".", doc, &["-o", "json", "-I0"]).unwrap();
        assert_eq!((stdout.as_str(), code), (*expected, 0), "{doc:?}");
    }
}

/// A sequence after a key that really is complete is still refused (#4530).
#[test]
fn test_a_sequence_after_a_completed_quoted_entry_stays_refused_6392() {
    let (stdout, code) = run_yq_stdin(
        ".",
        "- 'a ''b'' # c': v\n  - cell\n",
        &["-o", "json", "-I0"],
    )
    .unwrap();
    assert_eq!((stdout.as_str(), code), ("", 1));
}
