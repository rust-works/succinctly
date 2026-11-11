//! #6948: yq drops the non-specific tag (`!`, and its verbatim spelling `!<!>`) from its YAML output,
//! because the tag names no kind, so `a: ! x` prints `a: x` and a string written into `a: !` prints as
//! itself. `succinctly yq -o=yaml` printed the `!` back on every route, so `.x = "12"` over `x: !` read
//! back as the number `12`. A node's explicit tag is now the one the cursor reports, and that is none
//! for a non-specific one. Every row was captured from yq v4.53.3 and kept only when both tools agree
//! (a `tag` row over a custom tag read through a constructed array differs and is left out).

use anyhow::Result;

use crate::run_yq_stdin;

/// (document, filter, yq's stdout).
const ROWS: &[(&str, &str, &str)] = &[
    ("a: !\n", ".", "a:\n"),
    ("a: ! x\n", ".", "a: x\n"),
    ("a: ! 12\n", ".", "a: 12\n"),
    ("- ! x\n", ".", "- x\n"),
    ("a: !<!> x\n", ".", "a: x\n"),
    ("a: !\n", ".a = \"12\"", "a: \"12\"\n"),
    ("a: ! a\n", ".a = \"\"", "a: \"\"\n"),
    ("a: ! |\n  text\n", ".", "a: |\n  text\n"),
    ("? ! k\n: v\n", ".", "k: v\n"),
    ("! k: v\n", ".", "k: v\n"),
    ("a: ! [1, 2]\n", ".", "a: [1, 2]\n"),
    ("a: ! {b: 1}\n", ".", "a: {b: 1}\n"),
    ("a: ! x\n", ".a", "x\n"),
    ("a: ! x\n", ". as $x | $x", "a: x\n"),
    ("a: ! x\nb: 1\n", ".b = 2", "a: x\nb: 2\n"),
    ("a: ! x\n", "[.a]", "- x\n"),
    ("a: &r ! x\nb: *r\n", ".", "a: &r x\nb: *r\n"),
    ("a: ! x # c\n", ".", "a: x # c\n"),
    (
        "- !\n- ! |\n  x\n- ! >\n  y\n- ! \"q\"\n- ! [a]\n",
        ".",
        "-\n- |\n  x\n- >\n  y\n\n- \"q\"\n- [a]\n",
    ),
    ("- !\n- ! |\n  x\n", ".[1] = \"n\"", "-\n- |-\n  n\n"),
    (
        "a: ! x\nb: !custom y\nc: !!str z\n",
        ".",
        "a: x\nb: !custom y\nc: !!str z\n",
    ),
    ("a: ! 12\n", ".a + 1", "13\n"),
    ("a: ! x\n", ".a | tag", "!!str\n"),
    ("a: ! x\n", ".a | style", "\n"),
    ("a: ! x\n", ".. style=\"\"", "a: x\n"),
    ("a: ! x\nb: ! y\n", "sort_keys(.)", "a: x\nb: y\n"),
    ("a: ! x\n", "to_entries", "- key: a\n  value: x\n"),
    ("a: ! {b: ! 1}\n", ".a.b = 2", "a: {b: 2}\n"),
    ("a: ! x\n", ". * {\"c\": 1}", "a: x\nc: 1\n"),
    ("k: ! v\n", "with_entries(.)", "k: v\n"),
    ("a: ! x\n", "line", "1\n"),
    ("--- ! x\n", ".", "---\nx\n"),
    ("! x\n", ".", "x\n"),
    ("? ! 1\n: v\n", ".", "1: v\n"),
    ("! 1: v\n", ".", "1: v\n"),
    ("! 1: v\n", "keys", "- 1\n"),
    ("! true: v\n", "keys", "- true\n"),
    ("! null: v\n", "keys", "- null\n"),
    ("! 1: v\n", ".1", "v\n"),
    ("? ! 1\n: v\n", "to_entries", "- key: 1\n  value: v\n"),
    ("[! a, ! , ! 1]\n", ".", "[a, '', 1]\n"),
    ("{! a: ! b}\n", ".", "{a: b}\n"),
    ("a: &r ! x\nb: *r\n", ".b = \"y\"", "a: &r x\nb: y\n"),
    ("a: ! x\n---\nb: ! y\n", ".", "a: x\n---\nb: y\n"),
    ("! 1: v\n", ". | keys | .[0] | type", "!!int\n"),
];

#[test]
fn the_non_specific_tag_is_dropped_from_the_output_6948() -> Result<()> {
    for (doc, filter, expected) in ROWS {
        let (stdout, code) = run_yq_stdin(filter, doc, &[])?;
        assert_eq!(code, 0, "{filter} over {doc:?}");
        assert_eq!(&stdout, expected, "{filter} over {doc:?}");
    }
    Ok(())
}

/// A tag that names a kind or a custom one is still written.
#[test]
fn a_specific_tag_is_still_written_6948() -> Result<()> {
    let doc = "a: ! x\nb: !custom y\nc: !!str z\n";
    let (stdout, code) = run_yq_stdin(".", doc, &[])?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "a: x\nb: !custom y\nc: !!str z\n");
    Ok(())
}

/// `-P` forces the DOM route, which writes the node's tag from its presentation record.
#[test]
fn the_dom_route_drops_it_too_6948() -> Result<()> {
    let (stdout, code) = run_yq_stdin(".", "a: ! x\nb: ! [1]\n", &["-P"])?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "a: x\nb:\n  - 1\n");
    Ok(())
}
