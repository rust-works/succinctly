//! #6536: a `select` or `if` over a value a stage computed, whose condition does arithmetic
//! (`select(. % 3 == 0)`, `if .a % 2 == 0 then ..`), was answered through the reindex bridge: the
//! value written out as JSON text, indexed and read back per element. The pure-condition grammar
//! has an arithmetic arm now, which declines wherever the operator raises, so the bridge keeps jq's
//! message. Every row was captured from jq 1.7.1 (`-n -c`); a grid of 47 conditions, 6 program
//! shapes and 36 values (integers and floats, `nan`, the infinities, literals past 2^53, strings,
//! containers, `null`) gave no difference from it.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn arithmetic_in_a_select_or_if_condition_is_jqs_answer_6536() -> Result<()> {
    for (filter, expected) in [
        ("[range(10) | select(. % 3 == 0)]", "[0,3,6,9]"),
        ("[range(10) | select(. % 3 != 0 and . > 4)]", "[5,7,8]"),
        ("[range(10) | if . % 2 == 0 then . else empty end]", "[0,2,4,6,8]"),
        ("[range(6) | {a: .} | select(.a % 2 == 1) | .a]", "[1,3,5]"),
        ("[range(6) | {a: ., b: (. * 2)} | select(.a + .b == 6)]", "[{\"a\":2,\"b\":4}]"),
        ("[range(5) | select(. + 1 > 3)]", "[3,4]"),
        ("[range(5) | select((. + 1) * 2 > 4)]", "[2,3,4]"),
        ("[range(6) | select(. / 2 == 1)]", "[2]"),
        ("[range(5) | select(. * . == 4)]", "[2]"),
        ("[(0.5, 1.5, 2.5, -7, 7, -3.5, 1e1000, -1e1000) | select(. % 2 == 1)]", "[1.5,7,1E+1000]"),
        ("[(0.5, 2.5, 9007199254740993, 100000000000000000000) | select(. - 1 > 1)]", "[2.5,9007199254740993,100000000000000000000]"),
        ("[(\"a\", \"b\") | select(. + \"x\" == \"ax\")]", "[\"a\"]"),
        ("[(null, 1, [1]) | select(. + null == .)]", "[null,1,[1]]"),
        ("[(1, 2, 3) | select(. % 2 == 0 or . == 3)]", "[2,3]"),
        ("[range(4) | (. % 2 == 0)]", "[true,false,true,false]"),
        ("[range(4) | try select(. % 0 == 0) catch \"E\"]", "[\"E\",\"E\",\"E\",\"E\"]"),
        ("[range(4) | select(. % 0 == 0)?]", "[]"),
        ("[(\"a\", 1) | try select(. - 1 == 0) catch .]", "[\"string (\\\"a\\\") and number (1) cannot be subtracted\",1]"),
        ("[(1, [1], {}) | try select(. * {} == 1) catch .]", "[\"number (1) and object ({}) cannot be multiplied\",\"array ([1]) and object ({}) cannot be multiplied\"]"),
        ("[range(4) | select(. % 2 == 0) | . * 10]", "[0,20]"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-n", "-c", filter], None)?;
        assert_eq!(stdout, format!("{expected}\n"), "{filter}");
        assert_eq!(stderr, "", "{filter}");
        assert_eq!(code, 0, "{filter}");
    }
    Ok(())
}

#[test]
fn a_condition_whose_arithmetic_raises_keeps_jqs_error_6536() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-n", "-c", "[range(4) | select(. % 0 == 0)]"], None)?;
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        "jq: error (at <unknown>): number (0) and number (0) cannot be divided (remainder) because the divisor is zero\n"
    );
    assert_eq!(code, 5);
    Ok(())
}
