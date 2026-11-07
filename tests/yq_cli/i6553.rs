//! #6553: a block scalar header on a line of its own at the column of a bare `?` (`?`, `? &a`,
//! `- ?`, a comment aside) is the explicit key's content, as yq reads it (`?` / `|` / `  t` is
//! `{"t\n": null}`), like a block sequence at the `?`'s column is (#6370). It was refused as a line
//! with nothing to attach to. Every row was captured from yq v4.53.3: the bare `?` lines that stand
//! at columns 0, 2 and 4 with a `|`, `>` or `|-` header at the `?`'s own column, with and without
//! a `: v` value after the key (at the `?`'s column).

use anyhow::Result;

use crate::run_yq_stdin;

const ROWS: &[(&str, &str)] = &[
    ("?\n|\n  t\n", "{\"t\\n\":null}"),
    ("?\n|\n  t\n: v\n", "{\"t\\n\":\"v\"}"),
    ("?\n>\n  t\n", "{\"t\\n\":null}"),
    ("?\n>\n  t\n: v\n", "{\"t\\n\":\"v\"}"),
    ("?\n|-\n  t\n", "{\"t\":null}"),
    ("?\n|-\n  t\n: v\n", "{\"t\":\"v\"}"),
    ("? &a\n|\n  t\n", "{\"t\\n\":null}"),
    ("? &a\n|\n  t\n: v\n", "{\"t\\n\":\"v\"}"),
    ("? &a\n>\n  t\n", "{\"t\\n\":null}"),
    ("? &a\n>\n  t\n: v\n", "{\"t\\n\":\"v\"}"),
    ("? &a\n|-\n  t\n", "{\"t\":null}"),
    ("? &a\n|-\n  t\n: v\n", "{\"t\":\"v\"}"),
    ("? !!str\n|\n  t\n", "{\"t\\n\":null}"),
    ("? !!str\n|\n  t\n: v\n", "{\"t\\n\":\"v\"}"),
    ("? !!str\n>\n  t\n", "{\"t\\n\":null}"),
    ("? !!str\n>\n  t\n: v\n", "{\"t\\n\":\"v\"}"),
    ("? !!str\n|-\n  t\n", "{\"t\":null}"),
    ("? !!str\n|-\n  t\n: v\n", "{\"t\":\"v\"}"),
    ("? # c\n|\n  t\n", "{\"t\\n\":null}"),
    ("? # c\n|\n  t\n: v\n", "{\"t\\n\":\"v\"}"),
    ("? # c\n>\n  t\n", "{\"t\\n\":null}"),
    ("? # c\n>\n  t\n: v\n", "{\"t\\n\":\"v\"}"),
    ("? # c\n|-\n  t\n", "{\"t\":null}"),
    ("? # c\n|-\n  t\n: v\n", "{\"t\":\"v\"}"),
    ("- ?\n  |\n    t\n", "[{\"t\\n\":null}]"),
    ("- ?\n  |\n    t\n  : v\n", "[{\"t\\n\":\"v\"}]"),
    ("- ?\n  >\n    t\n", "[{\"t\\n\":null}]"),
    ("- ?\n  >\n    t\n  : v\n", "[{\"t\\n\":\"v\"}]"),
    ("- ?\n  |-\n    t\n", "[{\"t\":null}]"),
    ("- ?\n  |-\n    t\n  : v\n", "[{\"t\":\"v\"}]"),
    ("- ? &a\n  |\n    t\n", "[{\"t\\n\":null}]"),
    ("- ? &a\n  |\n    t\n  : v\n", "[{\"t\\n\":\"v\"}]"),
    ("- ? &a\n  >\n    t\n", "[{\"t\\n\":null}]"),
    ("- ? &a\n  >\n    t\n  : v\n", "[{\"t\\n\":\"v\"}]"),
    ("- ? &a\n  |-\n    t\n", "[{\"t\":null}]"),
    ("- ? &a\n  |-\n    t\n  : v\n", "[{\"t\":\"v\"}]"),
    ("- - ?\n    |\n      t\n", "[[{\"t\\n\":null}]]"),
    ("- - ?\n    |\n      t\n    : v\n", "[[{\"t\\n\":\"v\"}]]"),
    ("- - ?\n    >\n      t\n", "[[{\"t\\n\":null}]]"),
    ("- - ?\n    >\n      t\n    : v\n", "[[{\"t\\n\":\"v\"}]]"),
    ("- - ?\n    |-\n      t\n", "[[{\"t\":null}]]"),
    ("- - ?\n    |-\n      t\n    : v\n", "[[{\"t\":\"v\"}]]"),
    ("a:\n  ?\n  |\n    t\n", "{\"a\":{\"t\\n\":null}}"),
    ("a:\n  ?\n  |\n    t\n  : v\n", "{\"a\":{\"t\\n\":\"v\"}}"),
    ("a:\n  ?\n  >\n    t\n", "{\"a\":{\"t\\n\":null}}"),
    ("a:\n  ?\n  >\n    t\n  : v\n", "{\"a\":{\"t\\n\":\"v\"}}"),
    ("a:\n  ?\n  |-\n    t\n", "{\"a\":{\"t\":null}}"),
    ("a:\n  ?\n  |-\n    t\n  : v\n", "{\"a\":{\"t\":\"v\"}}"),
    ("a:\n  ? &a\n  |\n    t\n", "{\"a\":{\"t\\n\":null}}"),
    (
        "a:\n  ? &a\n  |\n    t\n  : v\n",
        "{\"a\":{\"t\\n\":\"v\"}}",
    ),
    ("a:\n  ? &a\n  >\n    t\n", "{\"a\":{\"t\\n\":null}}"),
    (
        "a:\n  ? &a\n  >\n    t\n  : v\n",
        "{\"a\":{\"t\\n\":\"v\"}}",
    ),
    ("a:\n  ? &a\n  |-\n    t\n", "{\"a\":{\"t\":null}}"),
    ("a:\n  ? &a\n  |-\n    t\n  : v\n", "{\"a\":{\"t\":\"v\"}}"),
    ("a: 1\n?\n|\n  t\n", "{\"a\":1,\"t\\n\":null}"),
    ("a: 1\n?\n|\n  t\n: v\n", "{\"a\":1,\"t\\n\":\"v\"}"),
    ("a: 1\n?\n>\n  t\n", "{\"a\":1,\"t\\n\":null}"),
    ("a: 1\n?\n>\n  t\n: v\n", "{\"a\":1,\"t\\n\":\"v\"}"),
    ("a: 1\n?\n|-\n  t\n", "{\"a\":1,\"t\":null}"),
    ("a: 1\n?\n|-\n  t\n: v\n", "{\"a\":1,\"t\":\"v\"}"),
];

#[test]
fn yq_reads_a_header_at_the_column_of_a_bare_question_mark_as_the_key_6553() -> Result<()> {
    for (doc, expected) in ROWS {
        let (stdout, status) = run_yq_stdin(".", doc, &["-o=json", "-I0"])?;
        assert_eq!((stdout.trim_end(), status), (*expected, 0), "{doc:?}");
    }
    Ok(())
}

/// A scalar that is not a block scalar header at the `?`'s column is still the next entry, and a
/// header one column to the right is still the key's content: neither moved.
#[test]
fn yq_keeps_the_other_scalars_at_a_bare_question_mark_s_column_6553() -> Result<()> {
    let (_stdout, status) = run_yq_stdin(".", "?\nfoo\n", &["-o=json", "-I0"])?;
    assert_eq!(status, 1);
    let (stdout, status) = run_yq_stdin(".", "?\n |\n   t\n", &["-o=json", "-I0"])?;
    assert_eq!((stdout.trim_end(), status), (r#"{"t\n":null}"#, 0));
    Ok(())
}
