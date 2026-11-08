//! #6335: a write prints an earlier entry of a repeated key the way it was written. The by-key
//! presentation tree describes only the live (last) entry of a key, so the entries before it were
//! printed with no spelling at all: `a:` (an empty value) came out `a: null`, `~`, `Null` and `True`
//! as `null`/`true`. Every row below was captured from yq v4.53.3 and runs through a route that
//! rebuilds the mapping (`=`, `|=`, `del`, `*`, `with_entries`, `to_entries | from_entries`).
//!
//! The rest of a shadow's presentation (a tag, quotes, a comment, an anchor) is still lost;
//! `docs/compliance/yq/limitations.md` records it.

use anyhow::Result;

use crate::run_yq_stdin;

/// `(document, filter, yq's output)`.
const ROWS: &[(&str, &str, &str)] = &[
    ("a:\na: 2\n", ".zzkey = 1", "a:\na: 2\nzzkey: 1\n"),
    ("a:\na: 2\n", ".[] |= .", "a:\na: 2\n"),
    ("a:\na: 2\n", "with_entries(.)", "a:\na: 2\n"),
    ("a:\na: 2\n", ". * {\"q\":1}", "a:\na: 2\nq: 1\n"),
    ("a:\na: 2\n", "del(.zzkey)", "a:\na: 2\n"),
    ("a:\na: 2\n", "(.. | select(. == 2)) = 5", "a:\na: 5\n"),
    ("a:\na: 2\n", "to_entries | from_entries", "a:\na: 2\n"),
    ("a:\na: 2\n", ".a = 7", "a:\na: 7\n"),
    ("a: 2\na:\n", ".zzkey = 1", "a: 2\na:\nzzkey: 1\n"),
    ("a:\na:\na: 3\n", ".zzkey = 1", "a:\na:\na: 3\nzzkey: 1\n"),
    ("a:\na:\na: 3\n", ".[] |= .", "a:\na:\na: 3\n"),
    ("a:\na:\na: 3\n", "with_entries(.)", "a:\na:\na: 3\n"),
    ("a:\na:\na: 3\n", ". * {\"q\":1}", "a:\na:\na: 3\nq: 1\n"),
    ("a:\na:\na: 3\n", "del(.zzkey)", "a:\na:\na: 3\n"),
    (
        "a:\na:\na: 3\n",
        "(.. | select(. == 2)) = 5",
        "a:\na:\na: 3\n",
    ),
    (
        "a:\na:\na: 3\n",
        "to_entries | from_entries",
        "a:\na:\na: 3\n",
    ),
    ("a:\na:\na: 3\n", ".a = 7", "a:\na:\na: 7\n"),
    ("a:\na:\n", ".zzkey = 1", "a:\na:\nzzkey: 1\n"),
    ("a: ~\na: 2\n", ".zzkey = 1", "a: ~\na: 2\nzzkey: 1\n"),
    ("a: Null\na: 2\n", ".zzkey = 1", "a: Null\na: 2\nzzkey: 1\n"),
    ("a: True\na: 2\n", ".zzkey = 1", "a: True\na: 2\nzzkey: 1\n"),
    (
        "p:\n  a:\n  a: 2\n  b:\nq: 1\n",
        ".zzkey = 1",
        "p:\n  a:\n  a: 2\n  b:\nq: 1\nzzkey: 1\n",
    ),
    (
        "a:\nb:\na: 2\nb: 3\n",
        ".zzkey = 1",
        "a:\nb:\na: 2\nb: 3\nzzkey: 1\n",
    ),
    (
        "a: 1\na: ~\na: 2\n",
        ".zzkey = 1",
        "a: 1\na: ~\na: 2\nzzkey: 1\n",
    ),
    (
        "a: 1\na: ~\na: 2\n",
        "with_entries(.)",
        "a: 1\na: ~\na: 2\n",
    ),
    (
        "a: 1\na: ~\na: 2\n",
        ". * {\"q\":1}",
        "a: 1\na: ~\na: 2\nq: 1\n",
    ),
    ("a: 1\na: ~\na: 2\n", "del(.zzkey)", "a: 1\na: ~\na: 2\n"),
    (
        "a: ~\na: 1\na: True\na: 2\n",
        ".zzkey = 1",
        "a: ~\na: 1\na: True\na: 2\nzzkey: 1\n",
    ),
    (
        "a: ~\na: 1\na: True\na: 2\n",
        "with_entries(.)",
        "a: ~\na: 1\na: True\na: 2\n",
    ),
    (
        "a: ~\na: 1\na: True\na: 2\n",
        ". * {\"q\":1}",
        "a: ~\na: 1\na: True\na: 2\nq: 1\n",
    ),
    (
        "a: ~\na: 1\na: True\na: 2\n",
        "del(.zzkey)",
        "a: ~\na: 1\na: True\na: 2\n",
    ),
    (
        "a:\nb:\na: ~\nb: ~\nz: 1\na: 2\nb: 3\n",
        ".zzkey = 1",
        "a:\nb:\na: ~\nb: ~\nz: 1\na: 2\nb: 3\nzzkey: 1\n",
    ),
    (
        "a:\nb:\na: ~\nb: ~\nz: 1\na: 2\nb: 3\n",
        "with_entries(.)",
        "a:\nb:\na: ~\nb: ~\nz: 1\na: 2\nb: 3\n",
    ),
    (
        "a:\nb:\na: ~\nb: ~\nz: 1\na: 2\nb: 3\n",
        ". * {\"q\":1}",
        "a:\nb:\na: ~\nb: ~\nz: 1\na: 2\nb: 3\nq: 1\n",
    ),
    (
        "a:\nb:\na: ~\nb: ~\nz: 1\na: 2\nb: 3\n",
        "del(.zzkey)",
        "a:\nb:\na: ~\nb: ~\nz: 1\na: 2\nb: 3\n",
    ),
    (
        "x:\n  a: ~\n  a: 1\ny:\n  a:\n  a:\n",
        ".zzkey = 1",
        "x:\n  a: ~\n  a: 1\ny:\n  a:\n  a:\nzzkey: 1\n",
    ),
    (
        "x:\n  a: ~\n  a: 1\ny:\n  a:\n  a:\n",
        "with_entries(.)",
        "x:\n  a: ~\n  a: 1\ny:\n  a:\n  a:\n",
    ),
    (
        "x:\n  a: ~\n  a: 1\ny:\n  a:\n  a:\n",
        ". * {\"q\":1}",
        "x:\n  a: ~\n  a: 1\ny:\n  a:\n  a:\nq: 1\n",
    ),
    (
        "x:\n  a: ~\n  a: 1\ny:\n  a:\n  a:\n",
        "del(.zzkey)",
        "x:\n  a: ~\n  a: 1\ny:\n  a:\n  a:\n",
    ),
];

#[test]
fn an_earlier_entry_of_a_repeated_key_keeps_its_spelling_6335() -> Result<()> {
    for (doc, filter, expected) in ROWS {
        let (out, code) = run_yq_stdin(filter, doc, &[])?;
        assert_eq!(
            (out.as_str(), code),
            (*expected, 0),
            "{filter} over {doc:?}"
        );
    }
    Ok(())
}

/// Both the empty value and a spelled one survive being the only thing a later entry changes.
#[test]
fn a_later_write_to_the_live_entry_leaves_the_shadow_spelled_6335() -> Result<()> {
    let (out, code) = run_yq_stdin(".a = 7", "a:\na: ~\na: 2\n", &[])?;
    assert_eq!((out.as_str(), code), ("a:\na: ~\na: 7\n", 0));
    Ok(())
}
