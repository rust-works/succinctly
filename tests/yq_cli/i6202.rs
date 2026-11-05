//! #6202: `pivot` reads its row nodes as they are, and an alias row (`*name`) is neither a sequence
//! nor a mapping to yq, so a pivot over one raises; `succinctly yq` resolved the alias and
//! transposed the anchor's value. The refusals are yq's, tag for tag (an alias has no tag), for an
//! alias row and for every other row that is not a sequence or mapping, an empty array and a pivot
//! of something that is not an array. Every expected message is what yq v4.53.3 prints.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

/// (document, expected stderr line).
const REFUSED: &[(&str, &str)] = &[
    // an alias row beside a sequence or mapping, either way round
    (
        "r: &row [a, b]\nl:\n  - *row\n  - [c]\n",
        "sequence contains elements of  and !!seq types",
    ),
    (
        "r: &row [a, b]\nl:\n  - [c]\n  - *row\n",
        "sequence contains elements of !!seq and  types",
    ),
    (
        "r: &row {x: 1}\nl:\n  - *row\n  - {x: 2}\n",
        "sequence contains elements of  and !!map types",
    ),
    (
        "r: &row {x: 1}\nl:\n  - {x: 2}\n  - *row\n",
        "sequence contains elements of !!map and  types",
    ),
    // an alias to an anchor on a row of the same list
    (
        "l:\n  - &r [a, b]\n  - *r\n",
        "sequence contains elements of !!seq and  types",
    ),
    // only aliases
    (
        "r: &row [a, b]\nl:\n  - *row\n  - *row\n",
        "can only pivot elements of !!seq or !!map types, received ",
    ),
    (
        "r: &row {x: 1}\nl:\n  - *row\n  - *row\n",
        "can only pivot elements of !!seq or !!map types, received ",
    ),
    // an alias beside a scalar
    (
        "r: &row [a, b]\nl:\n  - *row\n  - 3\n",
        "sequence contains elements of  and !!int types",
    ),
    // the other refusals, in yq's words
    (
        "l: [1, 2]\n",
        "can only pivot elements of !!seq or !!map types, received !!int",
    ),
    (
        "l: [a, b]\n",
        "can only pivot elements of !!seq or !!map types, received !!str",
    ),
    (
        "l: [null, null]\n",
        "can only pivot elements of !!seq or !!map types, received !!null",
    ),
    (
        "l: [!foo [a], !foo [b]]\n",
        "can only pivot elements of !!seq or !!map types, received !foo",
    ),
    (
        "l: []\n",
        "can only pivot elements of !!seq or !!map types, received ",
    ),
    (
        "l: [[a], {x: 1}]\n",
        "sequence contains elements of !!seq and !!map types",
    ),
    (
        "l: [{x: 1}, [a]]\n",
        "sequence contains elements of !!map and !!seq types",
    ),
    (
        "l: [[a], [b], 3]\n",
        "sequence contains elements of !!seq and !!int types",
    ),
    (
        "l: [1, 2.5]\n",
        "sequence contains elements of !!int and !!float types",
    ),
    (
        "l: [!foo [a], [b]]\n",
        "sequence contains elements of !foo and !!seq types",
    ),
    ("l: 5\n", "cannot pivot node of type !!int"),
    ("l: {x: 1}\n", "cannot pivot node of type !!map"),
    ("l: ~\n", "cannot pivot node of type !!null"),
];

#[test]
fn test_pivot_refuses_alias_rows_and_non_collection_rows_as_yq_does_6202() -> Result<()> {
    for &(document, message) in REFUSED {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(".l | pivot", document, &[])?;
        assert_ne!(code, 0, "{document:?}: {stdout}");
        assert!(stdout.is_empty(), "{document:?}: {stdout}");
        assert!(
            stderr.contains(&format!("Error: {message}\n")),
            "{document:?}: {stderr:?}"
        );
    }
    Ok(())
}

/// Rows that are sequences or mappings under their own tag, an anchor on a row (not an alias to
/// it), and a custom tag that is the same on every row: as before.
const PIVOTED: &[(&str, &str)] = &[
    ("l: [[a, b], [c]]\n", "[[\"a\",\"c\"],[\"b\",null]]\n"),
    ("l: [{x: 1}, {y: 2}]\n", "{\"x\":[1,null],\"y\":[null,2]}\n"),
    ("l: [[a], !!seq [b]]\n", "[[\"a\",\"b\"]]\n"),
    (
        "l: [{x: 1}, !!map {y: 2}]\n",
        "{\"x\":[1,null],\"y\":[null,2]}\n",
    ),
    (
        "l: [&r [a, b], [c, d]]\n",
        "[[\"a\",\"c\"],[\"b\",\"d\"]]\n",
    ),
    ("l: [[],[]]\n", "[]\n"),
];

#[test]
fn test_pivot_of_collection_rows_is_unchanged_6202() -> Result<()> {
    for &(document, expected) in PIVOTED {
        let (stdout, stderr, code) =
            run_yq_stdin_with_stderr(".l | pivot", document, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{document:?}: {stderr}");
        assert_eq!(stdout, expected, "{document:?}");
    }
    Ok(())
}

/// A computed array holds the same nodes in yq, aliases included, where here a collection of the
/// rows copies them and the alias is gone (the owned route keeps no alias mark, recorded in
/// `docs/compliance/yq/limitations.md`): the answer is the transposed anchor value, not yq's error.
#[test]
fn test_pivot_of_a_rebuilt_array_still_resolves_its_aliases_6202() -> Result<()> {
    let document = "r: &row [a, b]\nl:\n  - *row\n  - [c]\n";
    let (stdout, _, code) =
        run_yq_stdin_with_stderr("[.l[]] | pivot", document, &["-o=json", "-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "[[\"a\",\"c\"],[\"b\",null]]\n");
    Ok(())
}

/// The owned route (a literal, a variable, a rebuilt array) refuses in the same words: it has no
/// alias mark to read, but it has the tags.
const OWNED: &[(&str, &str)] = &[
    (
        "[1, 2.5] | pivot",
        "sequence contains elements of !!int and !!float types",
    ),
    (
        "[[1], {\"a\": 2}] | pivot",
        "sequence contains elements of !!seq and !!map types",
    ),
    (
        "[null, [1]] | pivot",
        "sequence contains elements of !!null and !!seq types",
    ),
    (
        "[1, 2] as $x | $x | pivot",
        "can only pivot elements of !!seq or !!map types, received !!int",
    ),
    (
        "[] | pivot",
        "can only pivot elements of !!seq or !!map types, received ",
    ),
    ("5 | pivot", "cannot pivot node of type !!int"),
    ("\"s\" | pivot", "cannot pivot node of type !!str"),
    ("null | pivot", "cannot pivot node of type !!null"),
    (
        "[.l[]] | pivot",
        "sequence contains elements of !!int and !!float types",
    ),
];

#[test]
fn test_pivot_of_a_computed_array_refuses_in_yqs_words_6202() -> Result<()> {
    for &(filter, message) in OWNED {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, "l: [1, 2.5]\n", &[])?;
        assert_ne!(code, 0, "{filter}: {stdout}");
        assert!(
            stderr.contains(&format!("Error: {message}\n")),
            "{filter}: {stderr:?}"
        );
    }
    Ok(())
}

/// `try` (a jq extension in yq mode) catches the refusal on either route.
#[test]
fn test_try_pivot_catches_the_refusal_6202() -> Result<()> {
    let (stdout, _, code) = run_yq_stdin_with_stderr(
        r#"[(.l | try pivot catch "document"), ([] | try pivot catch "owned")]"#,
        "l: [1, 2.5]\n",
        &["--jq-extensions", "-o=json", "-I=0"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "[\"document\",\"owned\"]\n");
    Ok(())
}
