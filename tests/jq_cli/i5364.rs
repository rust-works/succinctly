//! #5364: a `-0` integer spelling keeps its sign through the two- and three-argument math builtins,
//! as it does through the arithmetic operators and the one-argument builtins. `NumberLiteral(Int(0),
//! "-0")` is the one number whose sign lives only in its spelling, and `math_operand` read it as
//! `+0.0`. Expected values captured live against jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_stdin;

#[test]
fn test_negative_zero_integer_spelling_keeps_its_sign_in_binary_math_5364() -> Result<()> {
    for (filter, want) in [
        ("pow(-0;1)", "-0"),
        ("pow(1;-0)", "1"),
        ("[-0]|.[0]|pow(.;1)", "-0"),
        ("pow(-0;-1)", "-1.7976931348623157e+308"),
        ("pow(-1;-0)", "1"),
        ("[-0]|.[0]|pow(.;-1)", "-1.7976931348623157e+308"),
        ("pow(-0;3)", "-0"),
        ("pow(3;-0)", "1"),
        ("[-0]|.[0]|pow(.;3)", "-0"),
        ("atan2(-0;1)", "-0"),
        ("atan2(1;-0)", "1.5707963267948966"),
        ("[-0]|.[0]|atan2(.;1)", "-0"),
        ("atan2(-0;-1)", "-3.141592653589793"),
        ("atan2(-1;-0)", "-1.5707963267948966"),
        ("[-0]|.[0]|atan2(.;-1)", "-3.141592653589793"),
        ("atan2(-0;3)", "-0"),
        ("atan2(3;-0)", "1.5707963267948966"),
        ("[-0]|.[0]|atan2(.;3)", "-0"),
        ("fmod(-0;1)", "-0"),
        ("fmod(1;-0)", "null"),
        ("[-0]|.[0]|fmod(.;1)", "-0"),
        ("fmod(-0;-1)", "-0"),
        ("fmod(-1;-0)", "null"),
        ("[-0]|.[0]|fmod(.;-1)", "-0"),
        ("fmod(-0;3)", "-0"),
        ("fmod(3;-0)", "null"),
        ("[-0]|.[0]|fmod(.;3)", "-0"),
        ("fmin(-0;1)", "-0"),
        ("fmin(1;-0)", "-0"),
        ("[-0]|.[0]|fmin(.;1)", "-0"),
        ("fmin(-0;-1)", "-1"),
        ("fmin(-1;-0)", "-1"),
        ("[-0]|.[0]|fmin(.;-1)", "-1"),
        ("fmin(-0;3)", "-0"),
        ("fmin(3;-0)", "-0"),
        ("[-0]|.[0]|fmin(.;3)", "-0"),
        ("fmax(-0;1)", "1"),
        ("fmax(1;-0)", "1"),
        ("[-0]|.[0]|fmax(.;1)", "1"),
        ("fmax(-0;-1)", "-0"),
        ("fmax(-1;-0)", "-0"),
        ("[-0]|.[0]|fmax(.;-1)", "-0"),
        ("fmax(-0;3)", "3"),
        ("fmax(3;-0)", "3"),
        ("[-0]|.[0]|fmax(.;3)", "3"),
        ("ldexp(-0;1)", "-0"),
        ("ldexp(1;-0)", "1"),
        ("[-0]|.[0]|ldexp(.;1)", "-0"),
        ("ldexp(-0;-1)", "-0"),
        ("ldexp(-1;-0)", "-1"),
        ("[-0]|.[0]|ldexp(.;-1)", "-0"),
        ("ldexp(-0;3)", "-0"),
        ("ldexp(3;-0)", "3"),
        ("[-0]|.[0]|ldexp(.;3)", "-0"),
        ("scalb(-0;1)", "-0"),
        ("scalb(1;-0)", "1"),
        ("[-0]|.[0]|scalb(.;1)", "-0"),
        ("scalb(-0;-1)", "-0"),
        ("scalb(-1;-0)", "-1"),
        ("[-0]|.[0]|scalb(.;-1)", "-0"),
        ("scalb(-0;3)", "-0"),
        ("scalb(3;-0)", "3"),
        ("[-0]|.[0]|scalb(.;3)", "-0"),
        ("scalbln(-0;1)", "-0"),
        ("scalbln(1;-0)", "1"),
        ("[-0]|.[0]|scalbln(.;1)", "-0"),
        ("scalbln(-0;-1)", "-0"),
        ("scalbln(-1;-0)", "-1"),
        ("[-0]|.[0]|scalbln(.;-1)", "-0"),
        ("scalbln(-0;3)", "-0"),
        ("scalbln(3;-0)", "3"),
        ("[-0]|.[0]|scalbln(.;3)", "-0"),
        ("drem(-0;1)", "-0"),
        ("drem(1;-0)", "null"),
        ("[-0]|.[0]|drem(.;1)", "-0"),
        ("drem(-0;-1)", "-0"),
        ("drem(-1;-0)", "null"),
        ("[-0]|.[0]|drem(.;-1)", "-0"),
        ("drem(-0;3)", "-0"),
        ("drem(3;-0)", "null"),
        ("[-0]|.[0]|drem(.;3)", "-0"),
        ("remainder(-0;1)", "-0"),
        ("remainder(1;-0)", "null"),
        ("[-0]|.[0]|remainder(.;1)", "-0"),
        ("remainder(-0;-1)", "-0"),
        ("remainder(-1;-0)", "null"),
        ("[-0]|.[0]|remainder(.;-1)", "-0"),
        ("remainder(-0;3)", "-0"),
        ("remainder(3;-0)", "null"),
        ("[-0]|.[0]|remainder(.;3)", "-0"),
        ("fma(-0;1;0)", "0"),
        ("fma(1;-0;0)", "0"),
        ("fma(1;1;-0)", "1"),
        ("fma(-0;-1;-0)", "0"),
        ("fma(0;-1;-0)", "-0"),
        ("copysign(-0;1)", "0"),
        ("copysign(1;-0)", "-1"),
        ("copysign(-0;-1)", "-0"),
        ("copysign(-1;-0)", "-1"),
        ("copysign(-0;3)", "0"),
        ("copysign(3;-0)", "-3"),
        ("fdim(-0;1)", "0"),
        ("fdim(1;-0)", "1"),
        ("fdim(-0;-1)", "1"),
        ("fdim(-1;-0)", "0"),
        ("fdim(-0;3)", "0"),
        ("fdim(3;-0)", "3"),
        ("hypot(-0;1)", "1"),
        ("hypot(1;-0)", "1"),
        ("hypot(-0;-1)", "1"),
        ("hypot(-1;-0)", "1"),
        ("hypot(-0;3)", "3"),
        ("hypot(3;-0)", "3"),
        ("nextafter(-0;1)", "5e-324"),
        ("nextafter(1;-0)", "0.9999999999999999"),
        ("nextafter(-0;-1)", "-5e-324"),
        ("nextafter(-1;-0)", "-0.9999999999999999"),
        ("nextafter(-0;3)", "5e-324"),
        ("nextafter(3;-0)", "2.9999999999999996"),
        ("nexttoward(-0;1)", "5e-324"),
        ("nexttoward(1;-0)", "0.9999999999999999"),
        ("nexttoward(-0;-1)", "-5e-324"),
        ("nexttoward(-1;-0)", "-0.9999999999999999"),
        ("nexttoward(-0;3)", "5e-324"),
        ("nexttoward(3;-0)", "2.9999999999999996"),
        ("fma(-0;1;-0)", "-0"),
        ("fma(-0;-1;0)", "0"),
        ("fma(-0;0;-0)", "-0"),
        ("fma(1;-0;-0)", "-0"),
        ("fma(-1;0;-0)", "-0"),
        ("fma(-0;-0;0)", "0"),
        ("fma(-0;-0;-0)", "0"),
    ] {
        let (stdout, code) = run_jq_stdin(filter, "null", &["-n"])?;
        assert_eq!((stdout.trim_end(), code), (want.trim_end(), 0), "{filter}");
    }
    Ok(())
}

/// The issue's own repro: a `-0` that arrives from the *input*, not the filter text.
#[test]
fn test_negative_zero_from_the_input_keeps_its_sign_in_binary_math_5364() -> Result<()> {
    let (stdout, code) = run_jq_stdin(
        "pow(.;1), atan2(.;-1), fmod(.;3), ldexp(.;2)",
        "-0",
        &["-c"],
    )?;
    assert_eq!(
        (stdout.as_str(), code),
        ("-0\n-3.141592653589793\n-0\n-0\n", 0)
    );
    Ok(())
}

/// Spellings that were already right stay so: other zeros, and a `-0` that is the result of a
/// computation or a float spelling.
#[test]
fn test_other_zeros_are_unchanged_in_binary_math_5364() -> Result<()> {
    for (filter, want) in [
        ("pow(0;1)", "0"),
        ("pow(-0.0;1)", "-0"),
        ("pow(0*-1;1)", "-0"),
        ("atan2(0;-1)", "3.141592653589793"),
        ("atan2(-0.0;-1)", "-3.141592653589793"),
        ("fmod(0;3)", "0"),
        ("ldexp(0;2)", "0"),
    ] {
        let (stdout, code) = run_jq_stdin(filter, "null", &["-n"])?;
        assert_eq!((stdout.trim_end(), code), (want, 0), "{filter}");
    }
    Ok(())
}
