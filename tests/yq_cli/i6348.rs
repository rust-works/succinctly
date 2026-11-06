//! #6348: a block scalar header holds the indicators (one chomping character, one digit 1-9), blanks,
//! a comment and the line break, and nothing else. yq stops at anything more with `did not find
//! expected comment or line break` (`while scanning a block scalar`); `succinctly yq` dropped the text
//! and read the lines below as the content (`a: |  x` / `  y` was `{"a":"y\n"}`), and accepted a second
//! chomping character, a second digit and a `0` (`a: |--`, `a: |22`, `a: |0`). Every row was captured
//! from yq v4.53.3 (`yq -o=json -I0 .`).

use anyhow::Result;

use crate::{run_yq_stdin, run_yq_stdin_with_stderr};

const ARGS: &[&str] = &["-o=json", "-I=0"];

#[test]
fn test_text_after_a_block_scalar_indicator_is_refused_as_yq_refuses_it_6348() -> Result<()> {
    for doc in [
        "a: |  x\n  y\n",
        "a: | x\n  y\n",
        "a: |x\n  y\n",
        "a: |-x\n  y\n",
        "a: >-  x\n  y\n",
        "a: |-2x\n   y\n",
        "a: |\"q\"\n  y\n",
        "a: |,\n  x\n",
        "a: |\u{a0}\n  x\n",
        "|  x\n  y\n",
        "- |  x\n  y\n",
        "- |-x\n  y\n",
        "? |  x\n  y\n: 1\n",
        "a: |\n  x\nb: >  z\n  w\n",
        "a: !!str |  x\n  y\n",
        "a: &x |  x\n  y\n",
        "- !!str >  x\n  y\n",
        "- &a |--\n  y\n",
        "a:\n  b: |  q\n    r\n",
        // The line after a marker line is glued onto it (#6276), so `--- |` / `  x` is `--- |  x`.
        "--- |  x\n  y\n",
        "--- |\n  x\n",
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(".", doc, ARGS)?;
        assert_ne!(code, 0, "{doc:?} should be refused, got {stdout:?}");
        assert!(
            stderr.contains("block scalar header"),
            "{doc:?}: {stderr:?}"
        );
    }
    Ok(())
}

#[test]
fn test_a_repeated_or_zero_header_indicator_is_refused_as_yq_refuses_it_6348() -> Result<()> {
    for doc in [
        "a: |--\n  x\n",
        "a: |-+\n  x\n",
        "a: |+-\n  x\n",
        "a: |22\n  x\n",
        "a: |0\n  x\n",
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(".", doc, ARGS)?;
        assert_ne!(code, 0, "{doc:?} should be refused, got {stdout:?}");
        assert!(
            stderr.contains("block scalar header"),
            "{doc:?}: {stderr:?}"
        );
    }
    Ok(())
}

#[test]
fn test_a_header_that_holds_only_indicators_blanks_and_a_comment_still_reads_6348() -> Result<()> {
    for (doc, expected) in [
        ("a: |1-\n  x\n", "{\"a\":\" x\"}"),
        ("a: |-1\n  x\n", "{\"a\":\" x\"}"),
        ("a: |2-\n   x\n", "{\"a\":\" x\"}"),
        ("a: |1\n x\n", "{\"a\":\"x\\n\"}"),
        ("a: |#c\n  x\n", "{\"a\":\"x\\n\"}"),
        ("a: | #c\n  x\n", "{\"a\":\"x\\n\"}"),
        ("a: |\t# c\n  x\n", "{\"a\":\"x\\n\"}"),
        ("a: | # c\n  x\n", "{\"a\":\"x\\n\"}"),
        ("a: |  \n  x\n", "{\"a\":\"x\\n\"}"),
        ("a: |\t\n  x\n", "{\"a\":\"x\\n\"}"),
        ("a: |\r\n  x\r\n", "{\"a\":\"x\\n\"}"),
        ("a: |\r  x\r", "{\"a\":\"x\\n\"}"),
        ("a: >+\n  x\n\n", "{\"a\":\"x\\n\\n\"}"),
    ] {
        let (stdout, code) = run_yq_stdin(".", doc, ARGS)?;
        assert_eq!((stdout.trim_end(), code), (expected, 0), "{doc:?}");
    }
    Ok(())
}

/// Recorded divergence: U+0085, U+2028 and U+2029 end a line to yq as `\n` does, so a block scalar's
/// header ends at one and the lines below it are the content (`a: |<LS>` / `  x` is `"\nx\n"`). The
/// index does not model them as breaks (#4913), so the header's rest is not refused but the value is
/// wrong. Pinned to what succinctly answers today; delete a row when it starts to fail.
#[test]
fn test_recorded_divergence_a_unicode_break_ends_the_header_in_yq_6348() -> Result<()> {
    for (doc, expected) in [
        ("a: |\u{2028}\n  x\n", "{\"a\":\"x\\n\"}"),
        ("a: |\u{85}\n  x\n", "{\"a\":\"x\\n\"}"),
        // yq refuses this (`while scanning a simple key`); the rest of the line is dropped here.
        ("a: |\u{2028}garbage\n  x\n", "{\"a\":\"x\\n\"}"),
    ] {
        let (stdout, code) = run_yq_stdin(".", doc, ARGS)?;
        assert_eq!((stdout.trim_end(), code), (expected, 0), "{doc:?}");
    }
    Ok(())
}
