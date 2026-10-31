//! #5615: jq reads the digits of a number literal as a decNumber, so redundant leading zeros are
//! dropped before the spelling is kept (`00e1` is `0E+1`), and its lexer takes the longest match
//! (`1.`), leaving `.e1` to index the number. Every expected byte string was captured from the
//! pinned jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_leading_zero_literal_keeps_its_decnumber_spelling_5615() -> Result<()> {
    for (program, expected) in [
        ("00e1", "0E+1\n"),
        ("007e1", "7E+1\n"),
        ("0010e1", "1.0E+2\n"),
        ("0007e-1", "0.7\n"),
        ("000.50e2", "50\n"),
        // With an empty fraction too (#5614).
        ("00.e1", "0E+1\n"),
        ("007.e1", "7E+1\n"),
        ("000.E2", "0E+2\n"),
        ("[00.e1]", "[0E+1]\n"),
        ("007.50", "7.50\n"),
        ("00.5", "0.5\n"),
        // Controls: a spelling that was already right.
        ("007", "7\n"),
        ("00", "0\n"),
        ("0.0e1", "0\n"),
        ("[00e1,007e1]", "[0E+1,7E+1]\n"),
        ("00e1|tojson", "\"0E+1\"\n"),
        ("\"\\(00e1)\"", "\"0E+1\"\n"),
        // Arithmetic, comparison and negation see the value, not the spelling.
        ("00e1 == 0", "true\n"),
        ("007e1+0", "70\n"),
        ("-007e1", "-70\n"),
        ("-00e1", "-0\n"),
        ("-007.5e1|.+0", "-75\n"),
    ] {
        let (out, err, code) = run_jq_full(&["-nc", program], None)?;
        assert_eq!((out.as_str(), code), (expected, 0), "{program}: {err}");
    }
    Ok(())
}

#[test]
fn test_double_dot_after_a_number_indexes_it_5615() -> Result<()> {
    // `1.` is the number and `.e1` a field access on it: a runtime error, not a compile error.
    let (out, err, code) = run_jq_full(&["-nc", "1..e1"], None)?;
    assert_eq!((out.as_str(), code), ("", 5), "{err}");
    assert!(
        err.contains("Cannot index number with string \"e1\""),
        "{err}"
    );
    for (program, expected) in [
        (
            "try (1..e1) catch .",
            "\"Cannot index number with string \\\"e1\\\"\"\n",
        ),
        ("[1..e1?]", "[]\n"),
        // A negated literal: the postfix binds tighter than the `-`.
        ("-1..e1?", ""),
        (
            "try (-1..e1) catch .",
            "\"Cannot index number with string \\\"e1\\\"\"\n",
        ),
        ("[-1 .e1?]", "[]\n"),
        ("[-1..e1?]", "[]\n"),
        ("-1 .[0]?", ""),
    ] {
        let (out, err, code) = run_jq_full(&["-nc", program], None)?;
        assert_eq!((out.as_str(), code), (expected, 0), "{program}: {err}");
    }
    Ok(())
}

#[test]
fn test_double_dot_after_a_number_is_still_a_syntax_error_5615() -> Result<()> {
    // jq's lexer reads `1.` then `.`, which no grammar rule accepts.
    for program in ["1..", "[1..]", "1.5..e1"] {
        let (out, _err, code) = run_jq_full(&["-nc", program], None)?;
        assert_eq!((out.as_str(), code), ("", 3), "{program}");
    }
    Ok(())
}
