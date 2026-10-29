//! #5508: a number literal whose exponent is past decNumber's range prints the exponent jq's
//! decNumber settles on. A zero's exponent is raised or lowered to `[-1147483646, 999999999]`,
//! and a nonzero coefficient that underflows past `-1147483646` is rounded there, half-up on the
//! first digit dropped. Every row's expected text was captured from `/usr/bin/jq` 1.7.1
//! (`echo <literal> | jq -c .`).

use anyhow::Result;

use crate::run_jq_stdin;

#[test]
fn test_extreme_exponent_literal_prints_like_jq_5508() -> Result<()> {
    for (literal, expected) in [
        // The issue's table.
        ("0e999999999999999999999999", "0E+999999999"),
        ("0e9999999999", "0E+999999999"),
        ("0e1000000000", "0E+999999999"),
        ("0.0e9999999999", "0E+999999999"),
        ("-0e9999999999", "-0E+999999999"),
        ("1e-9999999999", "0E-1147483646"),
        ("1.5e-9999999999", "0E-1147483646"),
        ("0e-9999999999", "0E-1147483646"),
        // Already right before the change, so the clamp must start above them.
        ("1e-999999999", "1E-999999999"),
        ("1e-1000000000", "1E-1000000000"),
        ("0e-1000000000", "0E-1000000000"),
        ("0e999999999", "0E+999999999"),
        // The floor itself: kept at it, dropped below it, rounded half-up.
        ("1e-1147483646", "1E-1147483646"),
        ("1e-1147483647", "0E-1147483646"),
        ("5e-1147483647", "1E-1147483646"),
        ("-5e-1147483647", "-1E-1147483646"),
        ("4.99999e-1147483647", "0E-1147483646"),
        ("12.5e-1147483646", "1.3E-1147483645"),
        ("99.5e-1147483647", "1.0E-1147483645"),
        ("0.0e999999999", "0E+999999998"),
        ("0.00e-1147483646", "0E-1147483646"),
    ] {
        let (output, code) = run_jq_stdin(".", literal, &["-c"])?;
        assert_eq!(code, 0, "{literal}");
        assert_eq!(output.trim(), expected, "{literal}");
    }

    Ok(())
}

/// The text is the number's own rendering, so every route that prints a literal agrees.
#[test]
fn test_extreme_exponent_literal_agrees_on_every_print_route_5508() -> Result<()> {
    for (filter, expected) in [
        ("tostring", "\"0E-1147483646\""),
        ("[.]", "[0E-1147483646]"),
        ("tojson", "\"0E-1147483646\""),
        ("{a: .} | tojson", "\"{\\\"a\\\":0E-1147483646}\""),
    ] {
        let (output, code) = run_jq_stdin(filter, "1e-9999999999", &["-c"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(output.trim(), expected, "{filter}");
    }

    Ok(())
}
