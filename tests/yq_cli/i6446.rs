//! #6446: a folded scalar that starts with blank lines is written as yq writes it on the streaming route (`.`).
//! The folded writer widened the run of breaks the string starts with, which wrote one empty line too many: the
//! value `"\n\nx\n"` came out as `"\n\n\nx\n"` when read back (and `>+` of nothing but blank lines gained one). go-yaml
//! widens nothing for a string that starts with a break, and a value of nothing but breaks gets no trailing extra.
//!
//! What stays as it was is deliberate (ADR-0018, #836): yq writes an extra empty line before a *more-indented* line
//! of a folded scalar, which changes the value when it is read back (it is not idempotent), so the streaming route
//! keeps the lossless spelling there (`tests/yq_cli_tests.rs`, `test_block_scalar_folded_*`). Every row below is
//! one where yq's own output reads back as the same value, captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn a_folded_scalar_that_starts_with_blank_lines_is_written_as_yq_writes_it_6446() -> Result<()> {
    for (doc, args, want) in [
        ("a: >\n\n\n  x\n", &[][..], "a: >\n\n\n  x\n\n"),
        ("a: >\n\n\n  x\n", &["-I", "4"][..], "a: >\n\n\n    x\n\n"),
        ("a: >\n\n\n  a b\n", &[][..], "a: >\n\n\n  a b\n\n"),
        (
            "a: >\n\n\n  a b\n",
            &["-I", "4"][..],
            "a: >\n\n\n    a b\n\n",
        ),
        (
            "a: >\n\n\n\n  yy\nb: 1\n",
            &[][..],
            "a: >\n\n\n\n  yy\n\nb: 1\n",
        ),
        (
            "a: >\n\n\n\n  yy\nb: 1\n",
            &["-I", "4"][..],
            "a: >\n\n\n\n    yy\n\nb: 1\n",
        ),
        ("a: >\n\n      z\n\n", &[][..], "a: >\n\n  z\n\n"),
        ("a: >\n\n      z\n\n", &["-I", "4"][..], "a: >\n\n    z\n\n"),
        ("a: >\n\n    a b\n", &[][..], "a: >\n\n  a b\n\n"),
        ("a: >\n\n    a b\n", &["-I", "4"][..], "a: >\n\n    a b\n\n"),
        ("a: >-\n\n\n  x\nb: 1\n", &[][..], "a: >-\n\n\n  x\nb: 1\n"),
        (
            "a: >-\n\n\n  x\nb: 1\n",
            &["-I", "4"][..],
            "a: >-\n\n\n    x\nb: 1\n",
        ),
        ("a: >+\n\n", &[][..], "a: >+\n\n"),
        ("a: >+\n\n", &["-I", "4"][..], "a: >+\n\n"),
        ("a: >+\n\n\n", &[][..], "a: >+\n\n\n"),
        ("a: >+\n\n\n", &["-I", "4"][..], "a: >+\n\n\n"),
        (
            "x:\n  a: >\n\n\n    x\n  b: 1\n",
            &[][..],
            "x:\n  a: >\n\n\n    x\n\n  b: 1\n",
        ),
        (
            "x:\n  a: >\n\n\n    x\n  b: 1\n",
            &["-I", "4"][..],
            "x:\n    a: >\n\n\n        x\n\n    b: 1\n",
        ),
        ("- >\n\n\n  x\n- 2\n", &[][..], "- >\n\n\n  x\n\n- 2\n"),
        (
            "- >\n\n\n  x\n- 2\n",
            &["-I", "4"][..],
            "- >\n\n\n  x\n\n- 2\n",
        ),
        (
            "- a: >-\n\n    x\n  b: 1\n",
            &[][..],
            "- a: >-\n\n    x\n  b: 1\n",
        ),
        (
            "- a: >-\n\n    x\n  b: 1\n",
            &["-I", "4"][..],
            "- a: >-\n\n    x\n  b: 1\n",
        ),
        ("a: &k >\n\n\n  x\n", &[][..], "a: &k >\n\n\n  x\n\n"),
        (
            "a: &k >\n\n\n  x\n",
            &["-I", "4"][..],
            "a: &k >\n\n\n    x\n\n",
        ),
        ("a: !!str >\n\n  x\n", &[][..], "a: !!str >\n\n  x\n\n"),
        (
            "a: !!str >\n\n  x\n",
            &["-I", "4"][..],
            "a: !!str >\n\n    x\n\n",
        ),
        (
            "a: >\n\n\n  x\n\n  y\n",
            &[][..],
            "a: >\n\n\n  x\n\n  y\n\n",
        ),
        (
            "a: >\n\n\n  x\n\n  y\n",
            &["-I", "4"][..],
            "a: >\n\n\n    x\n\n    y\n\n",
        ),
        ("a: >\n  x\n\n  y\n", &[][..], "a: >\n  x\n\n  y\n\n"),
        (
            "a: >\n  x\n\n  y\n",
            &["-I", "4"][..],
            "a: >\n    x\n\n    y\n\n",
        ),
        (
            "a: >-\n  foo\n\n  bar\nc: after\n",
            &[][..],
            "a: >-\n  foo\n\n  bar\nc: after\n",
        ),
        (
            "a: >-\n  foo\n\n  bar\nc: after\n",
            &["-I", "4"][..],
            "a: >-\n    foo\n\n    bar\nc: after\n",
        ),
        ("a: >\n  x\n  y\n", &[][..], "a: >\n  x y\n\n"),
        ("a: >\n  x\n  y\n", &["-I", "4"][..], "a: >\n    x y\n\n"),
    ] {
        let (stdout, code) = run_yq_stdin(".", doc, args)?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{doc:?} {args:?}");
        // Whatever is written reads back as the value it was.
        let (value, _) = run_yq_stdin(".", doc, &["-o=json", "-I0"])?;
        let (again, _) = run_yq_stdin(".", &stdout, &["-o=json", "-I0"])?;
        assert_eq!(again, value, "{doc:?} {args:?}");
    }
    Ok(())
}

#[test]
fn a_folded_scalar_with_a_more_indented_line_still_reads_back_as_it_was_6446() -> Result<()> {
    // yq writes an extra empty line here and changes the value; this keeps it (see the module comment).
    for doc in [
        "a: >\n  x\n    y\n",
        "a: >-\n  x\n    y\n  z\n",
        "a: >\n\n\n  x\n    y\n",
        "x:\n  a: >\n    x\n      y\n  b: 1\n",
    ] {
        let (stdout, _) = run_yq_stdin(".", doc, &[])?;
        let (value, _) = run_yq_stdin(".", doc, &["-o=json", "-I0"])?;
        let (again, _) = run_yq_stdin(".", &stdout, &["-o=json", "-I0"])?;
        assert_eq!(again, value, "{doc:?}: {stdout:?}");
    }
    Ok(())
}
