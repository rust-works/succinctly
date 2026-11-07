//! #6540: a block scalar header on a line of its own, to the left of the key it would be the value
//! of (`- - k:` with the header at the second dash's column), is refused as yq refuses it
//! (`did not find expected '-' indicator`). The header at the key's own column or to its right is
//! the key's value, as before. Every row was captured from yq v4.53.3; the refused rows are the
//! whole grid of headers left of the key (one to three dashes, `|`, `>` and `|-`, with and without
//! a property after the colon).

use anyhow::Result;

use crate::{run_yq_split, run_yq_stdin};

const REFUSED: &[&str] = &[
    // Spacing after the dash, a comment after the colon, a plain mapping key, CRLF.
    "- -   k:\n  |\n    t\n",
    "-   k:\n |\n   t\n",
    "- - k: # c\n  |\n    t\n",
    "a:\n  b:\n|\n  t\n",
    "x:\n  k:\n |\n   t\n",
    "- - k:\r\n  |\r\n    t\r\n",
    "- k:\n|\n  t\n",
    "- k:\n |\n   t\n",
    "- k: &y\n|\n  t\n",
    "- k: &y\n |\n   t\n",
    "- k:\n>\n  t\n",
    "- k:\n >\n   t\n",
    "- k: &y\n>\n  t\n",
    "- k: &y\n >\n   t\n",
    "- k:\n|-\n  t\n",
    "- k:\n |-\n   t\n",
    "- k: &y\n|-\n  t\n",
    "- k: &y\n |-\n   t\n",
    "- - k:\n|\n  t\n",
    "- - k:\n |\n   t\n",
    "- - k:\n  |\n    t\n",
    "- - k:\n   |\n     t\n",
    "- - k: &y\n|\n  t\n",
    "- - k: &y\n |\n   t\n",
    "- - k: &y\n  |\n    t\n",
    "- - k: &y\n   |\n     t\n",
    "- - k:\n>\n  t\n",
    "- - k:\n >\n   t\n",
    "- - k:\n  >\n    t\n",
    "- - k:\n   >\n     t\n",
    "- - k: &y\n>\n  t\n",
    "- - k: &y\n >\n   t\n",
    "- - k: &y\n  >\n    t\n",
    "- - k: &y\n   >\n     t\n",
    "- - k:\n|-\n  t\n",
    "- - k:\n |-\n   t\n",
    "- - k:\n  |-\n    t\n",
    "- - k:\n   |-\n     t\n",
    "- - k: &y\n|-\n  t\n",
    "- - k: &y\n |-\n   t\n",
    "- - k: &y\n  |-\n    t\n",
    "- - k: &y\n   |-\n     t\n",
    "- - - k:\n|\n  t\n",
    "- - - k:\n |\n   t\n",
    "- - - k:\n  |\n    t\n",
    "- - - k:\n   |\n     t\n",
    "- - - k:\n    |\n      t\n",
    "- - - k:\n     |\n       t\n",
    "- - - k: &y\n|\n  t\n",
    "- - - k: &y\n |\n   t\n",
    "- - - k: &y\n  |\n    t\n",
    "- - - k: &y\n   |\n     t\n",
    "- - - k: &y\n    |\n      t\n",
    "- - - k: &y\n     |\n       t\n",
    "- - - k:\n>\n  t\n",
    "- - - k:\n >\n   t\n",
    "- - - k:\n  >\n    t\n",
    "- - - k:\n   >\n     t\n",
    "- - - k:\n    >\n      t\n",
    "- - - k:\n     >\n       t\n",
    "- - - k: &y\n>\n  t\n",
    "- - - k: &y\n >\n   t\n",
    "- - - k: &y\n  >\n    t\n",
    "- - - k: &y\n   >\n     t\n",
    "- - - k: &y\n    >\n      t\n",
    "- - - k: &y\n     >\n       t\n",
    "- - - k:\n|-\n  t\n",
    "- - - k:\n |-\n   t\n",
    "- - - k:\n  |-\n    t\n",
    "- - - k:\n   |-\n     t\n",
    "- - - k:\n    |-\n      t\n",
    "- - - k:\n     |-\n       t\n",
    "- - - k: &y\n|-\n  t\n",
    "- - - k: &y\n |-\n   t\n",
    "- - - k: &y\n  |-\n    t\n",
    "- - - k: &y\n   |-\n     t\n",
    "- - - k: &y\n    |-\n      t\n",
    "- - - k: &y\n     |-\n       t\n",
];

#[test]
fn yq_refuses_a_header_left_of_the_key_it_would_be_the_value_of_6540() -> Result<()> {
    for doc in REFUSED {
        let (stdout, status) = run_yq_stdin(".", doc, &["-o=json", "-I0"])?;
        assert_eq!((stdout.as_str(), status), ("", 1), "{doc:?}");
    }
    Ok(())
}

#[test]
fn yq_keeps_a_header_at_or_right_of_the_key_6540() -> Result<()> {
    for (doc, expected) in [
        ("- k:\n  |\n    t\n", "[{\"k\":\"t\\n\"}]"),
        ("- - k:\n    |\n      t\n", "[[{\"k\":\"t\\n\"}]]"),
        ("- - k: &y\n    |\n      t\n", "[[{\"k\":\"t\\n\"}]]"),
        ("- - - k:\n      |\n        t\n", "[[[{\"k\":\"t\\n\"}]]]"),
        ("a:\n|\n  t\n", "{\"a\":\"t\\n\"}"),
        (
            "a:\n  - - k:\n      |\n        t\n",
            "{\"a\":[[{\"k\":\"t\\n\"}]]}",
        ),
        ("- k:\n   |\n     t\n", "[{\"k\":\"t\\n\"}]"),
    ] {
        let (stdout, status) = run_yq_stdin(".", doc, &["-o=json", "-I0"])?;
        assert_eq!((stdout.trim_end(), status), (expected, 0), "{doc:?}");
    }
    Ok(())
}

/// yq's wording follows the container the key is in: a sequence item reads `did not find expected
/// '-' indicator`, a block mapping `did not find expected key`.
#[test]
fn yq_words_the_refusal_by_the_container_of_the_key_6540() -> Result<()> {
    for (doc, wording) in [
        (
            "- - k:\n  |\n    t\n",
            "did not find expected '-' indicator",
        ),
        ("a:\n  b:\n|\n  t\n", "did not find expected key"),
    ] {
        let (_stdout, stderr, status) = run_yq_split(".", doc, &["-o=json", "-I0"])?;
        assert_eq!(status, 1, "{doc:?}");
        assert!(stderr.contains(wording), "{doc:?}: {stderr}");
    }
    Ok(())
}
