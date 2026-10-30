//! #5593: `floor`, `ceil`, `round`, `trunc`, `sqrt` and `fabs` of a document number are answered
//! from the number itself instead of writing it out as JSON, indexing it and evaluating the
//! builtin over that one-number document. The answers are the ones the round trip gave, which are
//! jq's. Expected values were captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

const NUMBERS: &str = r"[0,-0,-0.0,1,-1,1.5,-1.5,2.5,-2.5,0.5,-0.5,0.49999999999999994,4.5,-4.5,1E5,1e-5,1e19,-1e20,100000000000000000000,123456789012345678901234567890,1.7976931348623157e308,1e1000,-1e1000,1e-400,9007199254740991,9007199254740992,9007199254740993,-9007199254740993,9223372036854775808,3.14159265358979323846,0.30000000000000004,16,2]";

#[test]
fn test_unary_math_of_document_numbers_matches_jq_5593() -> Result<()> {
    for (program, expected) in [
        (
            r"map(floor)",
            r"[0,-0,-0,1,-1,1,-2,2,-3,0,-1,0,4,-5,100000,0,1e+19,-1e+20,1e+20,123456789012345680000000000000,1.7976931348623157e+308,1.7976931348623157e+308,-1.7976931348623157e+308,0,9007199254740991,9007199254740992,9007199254740992,-9007199254740992,9223372036854776000,3,0,16,2]",
        ),
        (
            r"map(ceil)",
            r"[0,-0,-0,1,-1,2,-1,3,-2,1,-0,1,5,-4,100000,1,1e+19,-1e+20,1e+20,123456789012345680000000000000,1.7976931348623157e+308,1.7976931348623157e+308,-1.7976931348623157e+308,0,9007199254740991,9007199254740992,9007199254740992,-9007199254740992,9223372036854776000,4,1,16,2]",
        ),
        (
            r"map(round)",
            r"[0,-0,-0,1,-1,2,-2,3,-3,1,-1,0,5,-5,100000,0,1e+19,-1e+20,1e+20,123456789012345680000000000000,1.7976931348623157e+308,1.7976931348623157e+308,-1.7976931348623157e+308,0,9007199254740991,9007199254740992,9007199254740992,-9007199254740992,9223372036854776000,3,0,16,2]",
        ),
        (
            r"map(trunc)",
            r"[0,-0,-0,1,-1,1,-1,2,-2,0,-0,0,4,-4,100000,0,1e+19,-1e+20,1e+20,123456789012345680000000000000,1.7976931348623157e+308,1.7976931348623157e+308,-1.7976931348623157e+308,0,9007199254740991,9007199254740992,9007199254740992,-9007199254740992,9223372036854776000,3,0,16,2]",
        ),
        (
            r"map(sqrt)",
            r"[0,-0,-0,1,null,1.224744871391589,null,1.5811388300841898,null,0.7071067811865476,null,0.7071067811865475,2.1213203435596424,null,316.22776601683796,0.0031622776601683794,3162277660.1683793,null,10000000000,351364182882014.44,1.3407807929942596e+154,1.7976931348623157e+308,null,0,94906265.62425154,94906265.62425156,94906265.62425156,null,3037000499.97605,1.7724538509055159,0.5477225575051662,4,1.4142135623730951]",
        ),
        (
            r"map(fabs)",
            r"[0,0,0,1,1,1.5,1.5,2.5,2.5,0.5,0.5,0.49999999999999994,4.5,4.5,100000,1e-05,1e+19,1e+20,1e+20,123456789012345680000000000000,1.7976931348623157e+308,1.7976931348623157e+308,1.7976931348623157e+308,0,9007199254740991,9007199254740992,9007199254740992,9007199254740992,9223372036854776000,3.141592653589793,0.30000000000000004,16,2]",
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", program], Some(NUMBERS))?;
        assert_eq!(code, 0, "{program}: {stderr:?}");
        assert_eq!(stdout.trim_end(), expected, "{program}");
    }
    Ok(())
}

/// A value that is not a number keeps the evaluator's own error and wording.
#[test]
fn test_unary_math_of_a_non_number_still_raises_5593() -> Result<()> {
    for (program, expected) in [
        (
            r"map(try floor catch .)",
            r#"[1,"string (\"a\") number required","null (null) number required","boolean (true) number required","array ([1]) number required","object ({\"a\":2.5}) number required",-3]"#,
        ),
        (
            r"map(try ceil catch .)",
            r#"[2,"string (\"a\") number required","null (null) number required","boolean (true) number required","array ([1]) number required","object ({\"a\":2.5}) number required",-2]"#,
        ),
        (
            r"map(try round catch .)",
            r#"[2,"string (\"a\") number required","null (null) number required","boolean (true) number required","array ([1]) number required","object ({\"a\":2.5}) number required",-3]"#,
        ),
        (
            r"map(try trunc catch .)",
            r#"[1,"string (\"a\") number required","null (null) number required","boolean (true) number required","array ([1]) number required","object ({\"a\":2.5}) number required",-2]"#,
        ),
        (
            r"map(try sqrt catch .)",
            r#"[1.224744871391589,"string (\"a\") number required","null (null) number required","boolean (true) number required","array ([1]) number required","object ({\"a\":2.5}) number required",null]"#,
        ),
        (
            r"map(try fabs catch .)",
            r#"[1.5,"string (\"a\") number required","null (null) number required","boolean (true) number required","array ([1]) number required","object ({\"a\":2.5}) number required",2.5]"#,
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(
            &["-c", program],
            Some(r#"[1.5,"a",null,true,[1],{"a":2.5},-2.5]"#),
        )?;
        assert_eq!(code, 0, "{program}: {stderr:?}");
        assert_eq!(stdout.trim_end(), expected, "{program}");
    }
    Ok(())
}

/// The builtin's answer feeds the next stage as the number jq computed.
#[test]
fn test_unary_math_chains_with_other_stages_5593() -> Result<()> {
    for (program, expected) in [
        (r".a|map(floor|.+1)", r"[2,3,-3]"),
        (r".a|map(sqrt|floor)", r"[1,1,null]"),
        (r"[.a[]|ceil]|add", r"2"),
        (r"[.a[]|fabs|round]", r"[1,3,4]"),
    ] {
        let (stdout, stderr, code) =
            run_jq_full(&["-c", program], Some(r#"{"a":[1.2,2.8,-3.5]}"#))?;
        assert_eq!(code, 0, "{program}: {stderr:?}");
        assert_eq!(stdout.trim_end(), expected, "{program}");
    }
    Ok(())
}
