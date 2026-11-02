//! #5754: `strptime` reads `%E` and `%O` modifiers as the plain specifier, reads `%k` and `%l` as
//! the hour, and rejects a specifier the libc does not know and a `%` that ends the format, where
//! it once took both for a literal. Every expectation is what jq 1.7.1 (`/usr/bin/jq`) prints on
//! macOS, taking the first six fields (the weekday and yearday of a format that sets no date are
//! the platform libc's, #5340). The `%Ob`/`%Oh`/`%OB` month rows are the pin's libc: a glibc jq
//! reads the abbreviation with `%O` as it does without.

use anyhow::Result;

use crate::run_jq_null;

/// `$i | strptime($f) | .[:6]` as compact JSON, or `None` when jq rejects the text (exit 5).
fn strptime(input: &str, fmt: &str) -> Result<Option<String>> {
    let (out, code) = run_jq_null(
        "$i|strptime($f)|.[:6]",
        &["-c", "--arg", "i", input, "--arg", "f", fmt],
    )?;
    Ok(match code {
        0 => Some(out.trim_end().to_string()),
        5 => None,
        other => panic!("{input:?} | strptime({fmt:?}) exited {other}"),
    })
}

#[test]
fn a_modifier_reads_as_the_plain_specifier() -> Result<()> {
    for (input, fmt, want) in [
        ("2015", "%EY", "[2015,0,0,0,0,0]"),
        ("2015", "%OY", "[2015,0,0,0,0,0]"),
        ("15", "%Ey", "[2015,0,0,0,0,0]"),
        ("15", "%Oy", "[2015,0,0,0,0,0]"),
        ("20", "%EC", "[2000,0,0,0,0,0]"),
        ("03/05/15", "%Ex", "[2015,2,5,0,0,0]"),
        ("03/05/15", "%Ox", "[2015,2,5,0,0,0]"),
        ("Thu Mar  5 10:11:12 2015", "%Ec", "[2015,2,5,10,11,12]"),
        ("10:11:12", "%EX", "[1900,0,0,10,11,12]"),
        ("10:11:12", "%OX", "[1900,0,0,10,11,12]"),
        ("10:11:12 PM", "%Er", "[1900,0,0,22,11,12]"),
        ("0305", "%Em%Od", "[1900,2,5,0,0,0]"),
        ("03/05/15 2015", "%Ex %EY", "[2015,2,5,0,0,0]"),
        ("03/05/15 15", "%Ox %Oy", "[2015,2,5,0,0,0]"),
        ("2015-03-05", "%EF", "[2015,2,5,0,0,0]"),
        ("03/05/15", "%ED", "[2015,2,5,0,0,0]"),
        ("10:11", "%OH:%OM", "[1900,0,0,10,11,0]"),
        ("March", "%OB", "[1900,2,0,0,0,0]"),
        ("march", "%OB", "[1900,2,0,0,0,0]"),
        ("Thursday 5", "%EA %Oe", "[1900,0,5,0,0,0]"),
        ("9", "%k", "[1900,0,0,9,0,0]"),
        ("09", "%k", "[1900,0,0,9,0,0]"),
        ("23", "%k", "[1900,0,0,23,0,0]"),
        ("9:30", "%k:%M", "[1900,0,0,9,30,0]"),
        ("2024-03-05 9", "%Y-%m-%d %k", "[2024,2,5,9,0,0]"),
        ("9:30 PM", "%l:%M %p", "[1900,0,0,21,30,0]"),
        ("12:30 AM", "%l:%M %p", "[1900,0,0,0,30,0]"),
        ("0", "%l", "[1900,0,0,0,0,0]"),
    ] {
        assert_eq!(
            strptime(input, fmt)?.as_deref(),
            Some(want),
            "{input:?} | strptime({fmt:?})"
        );
    }
    Ok(())
}

#[test]
fn what_the_libc_does_not_read_is_no_match() -> Result<()> {
    for (input, fmt) in [
        ("Mar", "%Ob"),
        ("March", "%Ob"),
        ("March", "%Oh"),
        ("Mar", "%OB"),
        ("x", "%E"),
        ("%", "%E%"),
        ("x", "%EE"),
        ("5", "%E5"),
        ("2015", "%EOY"),
        ("x", "%Q"),
        ("5", "%5"),
        ("2015%", "%Y%"),
        ("%", "%"),
        ("03/05/15", "%x%E"),
        ("24", "%k"),
        ("13", "%l"),
        ("a", "%P"),
    ] {
        assert_eq!(strptime(input, fmt)?, None, "{input:?} | strptime({fmt:?})");
    }
    Ok(())
}
