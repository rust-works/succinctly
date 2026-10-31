//! #5651: yq compares an integer with a float through `strconv.ParseFloat` of each node's text,
//! and Go's `ParseFloat` refuses an integer written with a radix prefix, so `0x10 < 1.5` is
//! `strconv.ParseFloat: parsing "0x10": invalid syntax` rather than `false`. Two integers compare
//! as integers, and a decimal spelling (`007`, `+5`, `1e3`) parses. Every row was captured from
//! yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

const DOC: &str = "h: 0x10\nh2: 0xF\ni: 7\nf: 1.5\no: 0o17\np: +5\nz: 007\ne: 1e3\n";

/// `(filter, Ok(stdout) | Err(the text yq prints after "Error: "))`.
const ROWS: &[(&str, Result<&str, &str>)] = &[
    // An integer written with a radix prefix against a float, in either order and for every op.
    (
        ".h < .f",
        Err("strconv.ParseFloat: parsing \"0x10\": invalid syntax"),
    ),
    (
        ".f < .h",
        Err("strconv.ParseFloat: parsing \"0x10\": invalid syntax"),
    ),
    (
        ".h >= .f",
        Err("strconv.ParseFloat: parsing \"0x10\": invalid syntax"),
    ),
    (
        ".f <= .h",
        Err("strconv.ParseFloat: parsing \"0x10\": invalid syntax"),
    ),
    (
        ".o < .f",
        Err("strconv.ParseFloat: parsing \"0o17\": invalid syntax"),
    ),
    (
        ".h < .e",
        Err("strconv.ParseFloat: parsing \"0x10\": invalid syntax"),
    ),
    // A computed integer keeps the spelling, and a literal operand is read the same way.
    (
        "(.h + 0) < .f",
        Err("strconv.ParseFloat: parsing \"0x10\": invalid syntax"),
    ),
    (
        ".h < 1.5",
        Err("strconv.ParseFloat: parsing \"0x10\": invalid syntax"),
    ),
    (
        "select(.h < .f)",
        Err("strconv.ParseFloat: parsing \"0x10\": invalid syntax"),
    ),
    // A quoted spelling is a string, which no float compares with.
    (
        "\"0x10\" < .f",
        Err("!!str not yet supported for comparison"),
    ),
    // Two integers compare as integers, whatever their spelling.
    (".h < .i", Ok("false\n")),
    (".h < .h2", Ok("false\n")),
    (".h2 < .h", Ok("true\n")),
    (".o > .i", Ok("true\n")),
    // A decimal spelling parses.
    (".p < .f", Ok("false\n")),
    (".z < .f", Ok("false\n")),
    (".e < .f", Ok("false\n")),
    (".f < .e", Ok("true\n")),
    (".i > .f", Ok("true\n")),
    // `==` never goes through ParseFloat.
    (".h == .f", Ok("false\n")),
];

#[test]
fn test_a_radix_prefixed_integer_against_a_float_raises_parsefloat_5651() -> Result<()> {
    for (filter, expected) in ROWS {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, DOC, &[])?;
        match expected {
            Ok(want) => assert_eq!((stdout.as_str(), code), (*want, 0), "{filter}: {stderr:?}"),
            Err(message) => {
                assert_eq!(stdout, "", "{filter}");
                assert_eq!(stderr, format!("Error: {message}\n"), "{filter}");
                assert_eq!(code, 1, "{filter}");
            }
        }
    }
    Ok(())
}

/// jq mode has no such rule: it orders the two numbers by value.
#[test]
fn test_jq_mode_orders_a_hex_integer_against_a_float_by_value_5651() -> Result<()> {
    let (stdout, code) =
        crate::run_jq_stdin("[.h < .f, .f < .h]", "{\"h\":16,\"f\":1.5}", &["-c"])?;
    assert_eq!((stdout.as_str(), code), ("[false,true]\n", 0));
    Ok(())
}
