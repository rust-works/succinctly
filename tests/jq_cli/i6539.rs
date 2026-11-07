//! #6539: `map(body)` of an array a stage computed out of numbers (`[range(n) | . * 1.5] | map(. + 1)`,
//! the second `map` of `map(. + 1) | map(. + 1)` over a built array) was written out as JSON text,
//! indexed and parsed back by the reindex bridge. It is answered number by number now, for a body
//! with one plain answer per element, and the bridge keeps a body that raises, fans out or answers
//! a NaN, an infinity or a literal. (`.numbers | map(. + 1) | map(. + 1)` over a document array
//! never took the bridge: its first `map` is a lazy sequence the second reads.) Every row was
//! captured from jq 1.7.1 (`-n -c`); a grid of 12 arrays, 67 bodies and 11 continuations (8,844
//! programs) gave no difference from it but `map(range(.))` over a `1e308` element, which no run
//! of jq finishes.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn map_of_a_computed_number_array_is_jqs_answer_6539() -> Result<()> {
    for (filter, expected) in [
        ("[range(5) | . * 1.5] | map(. + 1)", "[1,2.5,4,5.5,7]"),
        ("[range(5) | . * 1.5] | map(. * 2) | add", "30"),
        ("[range(5) | . * 1.5] | map(. + 1) | map(. + 1) | add", "25"),
        (
            "[range(5) | . * 1.5] | map(. + 1) | map(. * 2) | map(. - 3)",
            "[-1,2,5,8,11]",
        ),
        ("[(1, 2, 3) | . + 0.5] | map(floor)", "[1,2,3]"),
        (
            "[(1, 2, 3) | . + 0.5] | map(sqrt) | map(. * 2)",
            "[2.449489742783178,3.1622776601683795,3.7416573867739413]",
        ),
        (
            "[range(4) | . * 1.5] | map(. > 2)",
            "[false,false,true,true]",
        ),
        ("[range(4) | . * 1.5] | map(. % 2)", "[0,1,1,0]"),
        (
            "[range(4) | . * 1.5] | map(tostring)",
            "[\"0\",\"1.5\",\"3\",\"4.5\"]",
        ),
        ("[range(4) | . * 1.5] | map([.])", "[[0],[1.5],[3],[4.5]]"),
        (
            "[range(4) | . * 1.5] | map({a: ., b: (. * 2)})",
            "[{\"a\":0,\"b\":0},{\"a\":1.5,\"b\":3},{\"a\":3,\"b\":6},{\"a\":4.5,\"b\":9}]",
        ),
        ("[range(4) | . * 1.5] | map(. as $x | $x * 2)", "[0,3,6,9]"),
        (
            "[range(4) | . * 1.5] | map(. / 4) | map(. * 4)",
            "[0,1.5,3,4.5]",
        ),
        ("[(0, -0, 1, -3) | . * 1] | map(. + 0)", "[0,0,1,-3]"),
        (
            "[(9007199254740993, 4611686018427387904) | . + 0] | map(. + 1)",
            "[9007199254740992,4611686018427388000]",
        ),
        (
            "[(1e308, 1e300) | . + 0] | map(. * 2) | map(. - 1e308)",
            "[1.7976931348623157e+308,-9.9999998e+307]",
        ),
        (
            "[range(3) | . * 2] | map(. * 1e308 * 10) | add",
            "1.7976931348623157e+308",
        ),
        (
            "[range(4) | . * 1.5] | map(. - 10 | sqrt) | map(. * 2)",
            "[null,null,null,null]",
        ),
        ("[range(4) | . * 1.5] | map(select(. > 1))", "[1.5,3,4.5]"),
        (
            "[range(4) | . * 1.5] | map(., .)",
            "[0,0,1.5,1.5,3,3,4.5,4.5]",
        ),
        ("[range(4) | . * 1.5] | map(empty)", "[]"),
        (
            "[range(3) | . * 1.5] | map(pow(.; 2)) | map(. + 1)",
            "[1,3.25,10]",
        ),
        (
            "[range(3) | . * 1.5] | map(. + 1) | map(. + 1) | sort | reverse",
            "[5,3.5,2]",
        ),
        (
            "[range(3) | . * 1.5] | map(. + 1) | map(. + 1) | tojson",
            "\"[2,3.5,5]\"",
        ),
        (
            "[range(3) | . * 1.5] | map(. + 1) | map(. + 1) | length",
            "3",
        ),
        (
            "[range(3) | . * 2] | map(. + 1) | map(. + 1) | [.[] | tostring] | join(\",\")",
            "\"2,4,6\"",
        ),
        (
            "[range(3) | . * 1.5] | map(try error(\"x\") catch .)",
            "[\"x\",\"x\",\"x\"]",
        ),
        (
            "[range(3) | . * 1.5] | map(. + 1) | map(. % 2 == 0)",
            "[false,true,true]",
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-n", "-c", filter], None)?;
        assert_eq!(stdout, format!("{expected}\n"), "{filter}");
        assert_eq!(stderr, "", "{filter}");
        assert_eq!(code, 0, "{filter}");
    }
    Ok(())
}

#[test]
fn a_map_body_that_raises_over_a_computed_number_array_keeps_jqs_error_6539() -> Result<()> {
    for (filter, message) in [
        ("[range(3) | . * 1.5] | map(. + 1) | map(. % 0)", "jq: error (at <unknown>): number (1) and number (0) cannot be divided (remainder) because the divisor is zero\n"),
        ("[range(3) | . * 2] | map(. + 1) | map(. + \"a\")", "jq: error (at <unknown>): number (1) and string (\"a\") cannot be added\n"),
        ("[range(3) | . * 2] | map(. + 1) | map(.a)", "jq: error (at <unknown>): Cannot index number with string \"a\"\n"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-n", "-c", filter], None)?;
        assert_eq!(stdout, "", "{filter}");
        assert_eq!(stderr, message, "{filter}");
        assert_eq!(code, 5, "{filter}");
    }
    Ok(())
}
