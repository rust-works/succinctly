//! #7315: a unary math builtin (`floor`, `ceil`, `round`, `trunc`, `sqrt`, `fabs`, `log`, ...) and
//! `pow`/`atan2` of a document number read the number's span once, from text it validated as a
//! number and as UTF-8 in one pass, where they used to decode it three times. Each row is
//! `(document, filter, stdout, stderr, exit status)` as jq 1.7.1 (`/usr/bin/jq`) answers: halves on
//! both sides of zero, every spelling of zero, `1.0`/`1.00`, exponents past a double's range, integers
//! past 2^53 and past `i64`, 17-digit and longer fractions (jq's 17-digit rounding, #2936), whitespace
//! inside the array, a filter after the builtin, and the non-numbers that raise. Libm digits that differ
//! between platforms (#3045) are kept out: every answer here is exact. The spans that are not
//! UTF-8 are in the second test.

use anyhow::Result;

use crate::{run_jq_binary_stdin, run_jq_full};

const ROWS: &[(&str, &str, &str, &str, i32)] = &[
    ("[1.5,-1.5,0.5,-0.5,2.5]", "[.[]|floor]", "[1,-2,0,-1,2]\n", "", 0),
    ("[1.5,-1.5,0.5,-0.5,2.5]", "[.[]|ceil]", "[2,-1,1,-0,3]\n", "", 0),
    ("[1.5,-1.5,0.5,-0.5,2.5]", "[.[]|round]", "[2,-2,1,-1,3]\n", "", 0),
    ("[1.5,-1.5,0.5,-0.5,2.5]", "[.[]|trunc]", "[1,-1,0,-0,2]\n", "", 0),
    ("[-0,0,-0.0,1.0,1.00,100]", "[.[]|floor]", "[-0,0,-0,1,1,100]\n", "", 0),
    ("[-0,0,-0.0,1.0,1.00,100]", "[.[]|fabs]", "[0,0,0,1,1,100]\n", "", 0),
    ("[-0,0,-0.0,1.0,1.00,100]", "[.[]|sqrt]", "[-0,0,-0,1,1,10]\n", "", 0),
    ("[1e1000,-1e1000,1e-400,5e-324]", "[.[]|floor]", "[1.7976931348623157e+308,-1.7976931348623157e+308,0,0]\n", "", 0),
    ("[1e1000,-1e1000,1e-400,5e-324]", "[.[]|sqrt]", "[1.7976931348623157e+308,null,0,2.2227587494850775e-162]\n", "", 0),
    ("[12345678901234567890,123456789012345678901234567890,9007199254740993,-9223372036854775809]", "[.[]|floor]", "[12345678901234567000,123456789012345680000000000000,9007199254740992,-9223372036854776000]\n", "", 0),
    ("[12345678901234567890,123456789012345678901234567890,9007199254740993,-9223372036854775809]", "[.[]|fabs]", "[12345678901234567000,123456789012345680000000000000,9007199254740992,9223372036854776000]\n", "", 0),
    ("[1.7976931348623157e308,2.7293109604053567083,0.30000000000000004]", "[.[]|floor]", "[1.7976931348623157e+308,2,0]\n", "", 0),
    ("[1.7976931348623157e308,2.7293109604053567083,0.30000000000000004]", "[.[]|fabs]", "[1.7976931348623157e+308,2.7293109604053565,0.30000000000000004]\n", "", 0),
    ("[1,1.0,-0]", "[.[]|log]", "[0,0,-1.7976931348623157e+308]\n", "", 0),
    ("[0.5,3,-0]", "[.[]|pow(.;2)]", "[0.25,9,0]\n", "", 0),
    ("[0,-0,0.0]", "[.[]|atan2(.;1)]", "[0,-0,0]\n", "", 0),
    ("[1E2,1e+2,1e-2,-1E-2]", "[.[]|ceil]", "[100,100,1,-0]\n", "", 0),
    ("[0.1, 0.2 ,\n3.5]", "map(floor)", "[0,0,3]\n", "", 0),
    ("[1.5,2.5]", "[.[]|floor|.+1]", "[2,3]\n", "", 0),
    ("[1.5,\"1.5\",null]", "[.[]|try floor catch .]", "[1,\"string (\\\"1.5\\\") number required\",\"null (null) number required\"]\n", "", 0),
    ("[1.5,\"1.5\",null]", "[.[]|sqrt?]", "[1.224744871391589]\n", "", 0),
    ("[\"1.5\"]", "[.[]|floor]", "", "jq: error (at <stdin>:0): string (\"1.5\") number required\n", 5),
    ("[true]", "[.[]|ceil]", "", "jq: error (at <stdin>:0): boolean (true) number required\n", 5),
    ("[\"\\u00e9\"]", "[.[]|fabs]", "", "jq: error (at <stdin>:0): string (\"é\") number required\n", 5),
];

#[test]
fn test_math_builtins_of_document_numbers_answer_as_jq_7315() -> Result<()> {
    for (document, filter, stdout, stderr, code) in ROWS {
        let (out, err, status) = run_jq_full(&["-c", filter], Some(document))?;
        assert_eq!(
            (out.as_str(), err.as_str(), status),
            (*stdout, *stderr, *code),
            "{filter} over {document:?}"
        );
    }
    Ok(())
}

/// A number span holding bytes that are not UTF-8 is a parse error before any builtin runs, and a
/// string that is not UTF-8 is still the builtin's error, as in jq 1.7.1 (exit 5, empty stdout).
#[test]
fn test_math_builtin_over_non_utf8_bytes_fails_as_jq_7315() -> Result<()> {
    for input in [
        &b"[1\xff]"[..],
        b"[\xff1]",
        b"[-\xc3\xa9]",
        b"[\"\xff\", 1.5]",
        b"[1.5, \"a\xffb\"]",
    ] {
        for filter in ["[.[]|floor]", "[.[]|sqrt]", "[.[]|pow(.;2)]"] {
            let (out, status) = run_jq_binary_stdin(filter, input, &["-c"])?;
            assert_eq!(
                (out.as_slice(), status),
                (&b""[..], 5),
                "{filter} over {:?}",
                String::from_utf8_lossy(input)
            );
        }
    }
    Ok(())
}
