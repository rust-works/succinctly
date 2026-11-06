//! #6379: what yq refuses after an explicit-key or explicit-value indicator, in the positions the first
//! slice (the `? ` after an implicit key's colon) left. Every row was captured from yq v4.53.3
//! (`yq -o=json -I0 .`).
//!
//! - A node property before a `?` opens no key, wherever it stands: `&t ? x`, `- &t ? x`, `? - &t ? x`
//!   are `mapping keys are not allowed in this context`. A property on a line of its own, and `&t ?x`
//!   (no blank after the `?`), are not that.
//! - go-yaml skips no tab where a simple key is allowed, which is right after the `?` of a key and the
//!   `:` of an explicit value: `?\tx` and `:\tb` are `found character that cannot start any token`. A
//!   tab among the blanks that end at a `#` is its comment scanner's, and is read.

use anyhow::Result;

use crate::{run_yq_stdin, run_yq_stdin_with_stderr};

const ARGS: &[&str] = &["-o=json", "-I=0"];

fn refused(doc: &str, reason: &str) -> Result<()> {
    let (out, err, code) = run_yq_stdin_with_stderr(".", doc, ARGS)?;
    assert_eq!(out, "", "{doc:?}");
    assert_ne!(code, 0, "{doc:?}");
    // CI appends a backtrace after the first line, so only that line is read.
    let first = err.lines().next().unwrap_or_default();
    assert!(first.contains(reason), "{doc:?}: {err}");
    Ok(())
}

#[test]
fn test_a_property_before_an_explicit_key_indicator_is_refused_6379() -> Result<()> {
    for doc in [
        "&t ? x\n",
        "!!str ? x\n",
        "&t ? x\n: v\n",
        "- &t ? x\n",
        "- !!str ? x\n",
        "- &t !!str ? x\n",
        "- !!str &t ? x\n",
        "- &t ? x\n  : v\n",
        "k:\n  - &t ? x\n",
        "a:\n  - &t ? x\n  - y\n",
        "- - &t ? x\n",
        "? - &t ? x\n: v\n",
        "? - !!str ? x\n: v\n",
        "? - &t !!str ? x\n: v\n",
        "? - - &t ? x\n: v\n",
        "- &t ?\n",
        "- &t ?\n  x\n",
        "? - &t ?\n: v\n",
        "? &t ? x\n: v\n",
    ] {
        // The row `? &t ? x` was already refused, by #6340's older message (`mapping key is not
        // allowed ...`), so only what both spellings share is read.
        refused(doc, "not allowed in this context")?;
    }
    Ok(())
}

#[test]
fn test_a_tab_after_an_explicit_key_or_value_indicator_is_refused_6379() -> Result<()> {
    for doc in [
        "?\tx\n: v\n",
        "?\tx\n",
        "- ?\tx\n",
        "a:\n  ?\tx\n  : v\n",
        "?\t\n: v\n",
        "? \tx\n: v\n",
        "?\t- a\n",
        "?\t",
        "? a\n:\tb\n",
        "? a\n:\t\n",
        "? a\n:\t- b\n",
        "? a\n: \tb\n",
        "? a\n:\t",
        "? x\n:\t[a]\n",
    ] {
        refused(doc, "found character that cannot start any token")?;
    }
    Ok(())
}

#[test]
fn test_a_property_on_its_own_line_or_a_tab_before_a_comment_is_read_as_before_6379() -> Result<()>
{
    for (doc, expected) in [
        ("- &t\n  ? x\n", "[{\"x\":null}]\n"),
        ("- &t ?x\n", "[\"?x\"]\n"),
        ("? a\n:\t# c\n", "{\"a\":null}\n"),
        ("- ?\t# c\n", "[{\"\":null}]\n"),
        ("? ?\t# c\n: v\n", "{\"\":\"v\"}\n"),
        ("? a\n: ?\t# c\n", "{\"a\":{\"\":null}}\n"),
        ("?\t# c\n: v\n", "{\"\":\"v\"}\n"),
        ("a:\tb\n", "{\"a\":\"b\"}\n"),
        ("? - ? x\n: v\n", "{\"\":\"v\"}\n"),
    ] {
        let (stdout, code) = run_yq_stdin(".", doc, ARGS)?;
        assert_eq!((stdout.as_str(), code), (expected, 0), "{doc:?}");
    }
    Ok(())
}
