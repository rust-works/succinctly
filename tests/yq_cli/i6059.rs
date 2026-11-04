//! #6059: go-yaml refuses a plain scalar that continues on a line whose leading blanks hold a tab at a
//! column below the enclosing block's indent plus one (`found a tab character that violates
//! indentation`), in a flow collection as in block context. A flow collection at the document root has
//! no block indent to violate. Every row was captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn a_tab_that_indents_a_flow_plain_scalar_continuation_is_refused_6059() -> Result<()> {
    for input in [
        "a: [x\n\ty]\n",
        "a: [x \n\ty]\n",
        "a: [x\n\t y]\n",
        "a: [x\n\t\n y]\n",
        "a: [x\n \n\ty]\n",
        "a: [x\n\n\ty]\n",
        "a: [x\n\t]\n",
        "a: [x\n\t,y]\n",
        "a: [x\r\n\ty]\n",
        "a: [x\n\ty\n\tz]\n",
        "a: [[x\n\ty]]\n",
        "a: {k: x\n\ty}\n",
        "a: [{k: v\n\tw}]\n",
        "- [x\n\ty]\n",
        "- a: [x\n\ty]\n",
        "? [x\n\ty]\n: 1\n",
        "a:\n  b: [x\n \ty]\n",
        "a:\n  - [x\n \ty]\n",
        "a: {? x\n\ty : 1}\n",
        "a: [? x\n\ty : 1]\n",
    ] {
        let (out, code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        assert_eq!(out.trim_end(), "", "{input:?}");
        assert_ne!(code, 0, "{input:?}");
    }
    Ok(())
}

#[test]
fn a_tab_deeper_than_the_block_indent_or_outside_a_plain_scalar_is_read_6059() -> Result<()> {
    for (input, expected) in [
        ("a: [x\n y]\n", "{\"a\":[\"x y\"]}"),
        ("a: [x\n  \ty]\n", "{\"a\":[\"x y\"]}"),
        ("a:\n  b: [x\n   \ty]\n", "{\"a\":{\"b\":[\"x y\"]}}"),
        ("a:\n  - [x\n   \ty]\n", "{\"a\":[[\"x y\"]]}"),
        ("a:\n  - [x\n    \ty]\n", "{\"a\":[[\"x y\"]]}"),
        ("[x\n\ty]\n", "[\"x y\"]"),
        ("[x\n\n\ty]\n", "[\"x\\ny\"]"),
        ("a: [x,\n\ty]\n", "{\"a\":[\"x\",\"y\"]}"),
        ("a: [\"x\n\ty\"]\n", "{\"a\":[\"x y\"]}"),
        ("a: [x, 'y\n\tz']\n", "{\"a\":[\"x\",\"y z\"]}"),
        ("k: [a,\n\tb]\n", "{\"k\":[\"a\",\"b\"]}"),
    ] {
        let (out, code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        assert_eq!((out.trim_end(), code), (expected, 0), "{input:?}");
    }
    Ok(())
}

/// The check looks at a bounded number of lines per break: a run of blank lines inside a flow
/// scalar (100,000 of them, 0.01 s) was quadratic in a first version of the check.
#[test]
fn a_long_run_of_blank_lines_in_a_flow_scalar_stays_linear_6059() -> Result<()> {
    let input = format!("a:\n  b: [x{} y]\n", "\n".repeat(100_000));
    let (out, code) = run_yq_stdin(".a.b[0] | length", &input, &["-o=json", "-I=0"])?;
    assert_eq!((out.trim_end(), code), ("100001", 0));
    Ok(())
}

/// A tab on the line after a run of blank lines is still refused.
#[test]
fn a_tab_after_a_run_of_blank_lines_is_still_refused_6059() -> Result<()> {
    for blanks in [1, 2, 3, 7] {
        let input = format!("a:\n  b: [x{}\ty]\n", "\n".repeat(blanks));
        let (out, code) = run_yq_stdin(".", &input, &["-o=json", "-I=0"])?;
        assert_eq!(out.trim_end(), "", "{blanks} blank lines");
        assert_ne!(code, 0, "{blanks} blank lines");
    }
    Ok(())
}
