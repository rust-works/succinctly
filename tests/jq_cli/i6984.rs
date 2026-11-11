//! #6984: a computed slice bound below zero with a fractional part is added to the length in double
//! arithmetic before it is rounded, as jq's `parse_slice` does. `floor(f + len) == floor(f) + len` holds
//! for real numbers but not for the rounded sum: `-1e-66 + 5` is exactly `5.0`, so
//! `[1,2,3,4,5] | .[-1e-66:]` is `[]` where `floor` first made it `[5]`, and `-1.0000000000000002 + 5`
//! is `4.0`, so that start is `4`, not `3`. The end bound goes through the same sum (the past-the-end
//! answer #3404 gave an end in `(-1, 0)` is right for `-0.5` and wrong for `-0.9999999999999999`).
//! Every row was captured from jq 1.7.1; the path, write and `del` rows already agreed and pin that
//! they still do.

use anyhow::Result;

use crate::run_jq_full;

/// `(input, filter, stdout)`: the filter runs with `-c` over `input`.
const ROWS: &[(&str, &str, &str)] = &[
    ("-1e-66", "[1,2,3,4,5] | .[-1e-66:]", "[]\n"),
    ("-1e-66", ". as $n | [1,2,3,4,5] | .[$n:]", "[]\n"),
    ("-1e-66", ". as $n | [1,2,3,4,5] | .[$n:5]", "[]\n"),
    ("-1e-66", ". as $n | [1,2,3,4,5] | [.[$n:]]", "[[]]\n"),
    ("-1e-66", ". as $n | \"abcde\" | .[$n:]", "\"\"\n"),
    (
        "-1e-66",
        ". as $n | [1,2,3,4,5] | .[{\"start\":$n,\"end\":null}]",
        "[]\n",
    ),
    ("-1e-66", ". as $n | [1,2,3,4,5] | .[$n:][0]", "null\n"),
    ("-1e-66", ". as $n | [1,2,3,4,5] | .[$n:] | length", "0\n"),
    ("-1e-66", ". as $n | [[1,2,3,4,5]] | .[0][$n:]", "[]\n"),
    (
        "-1e-66",
        ". as $n | [1,2,3,4,5] | to_entries | .[$n:]",
        "[]\n",
    ),
    (
        "-1e-66",
        ". as $n | [range(1;1000001)] | .[($n*1e-5):] | length",
        "0\n",
    ),
    (
        "{\"n\":-1e-66,\"a\":[1,2,3,4,5],\"s\":\"abcde\"}",
        ".n as $n | [.a[$n:], .s[$n:], .a[$n:3], .a[{\"start\":$n,\"end\":null}]]",
        "[[],\"\",[],[]]\n",
    ),
    (
        "-1e-66",
        ". as $n | [1,2,3,4,5] | .[$n:] |= [\"x\"]",
        "[1,2,3,4,5,\"x\"]\n",
    ),
    (
        "-1e-66",
        ". as $n | [1,2,3,4,5] | del(.[$n:])",
        "[1,2,3,4,5]\n",
    ),
    (
        "-1e-66",
        ". as $n | [1,2,3,4,5] | path(.[$n:][0])",
        "[{\"start\":-1E-66,\"end\":null},0]\n",
    ),
    (
        "-1.0000000000000002",
        ". as $n | [1,2,3,4,5] | .[$n:]",
        "[5]\n",
    ),
    (
        "-1.0000000000000002",
        ". as $n | [1,2,3,4,5] | .[:$n]",
        "[1,2,3,4]\n",
    ),
    (
        "-0.9999999999999999",
        ". as $n | [1,2,3,4,5] | .[:$n]",
        "[1,2,3,4]\n",
    ),
    (
        "-0.9999999999999999",
        ". as $n | \"abcde\" | .[:$n]",
        "\"abcd\"\n",
    ),
    (
        "-0.9999999999999999",
        ". as $n | [1,2,3,4,5] | .[$n:]",
        "[5]\n",
    ),
    (
        "-0.5",
        ". as $n | [1,2,3,4,5] | [.[$n:], .[:$n]]",
        "[[5],[1,2,3,4,5]]\n",
    ),
    (
        "-1.5",
        ". as $n | [1,2,3,4,5] | [.[$n:], .[:$n]]",
        "[[4,5],[1,2,3,4]]\n",
    ),
    (
        "-1e-17",
        ". as $n | [range(0;4)] | [.[$n:], .[:$n]]",
        "[[],[0,1,2,3]]\n",
    ),
    (
        "-1e-17",
        ". as $n | [range(1;1000001)] | [(.[$n:] | length), (.[:$n] | length)]",
        "[0,1000000]\n",
    ),
    (
        "-4503599627370495.5",
        ". as $n | [1,2,3,4,5] | [.[$n:], .[:$n]]",
        "[[1,2,3,4,5],[]]\n",
    ),
    (
        "-1e-10",
        ". as $n | [1,2,3,4,5] | [.[$n:], .[:$n]]",
        "[[5],[1,2,3,4,5]]\n",
    ),
    (
        "-1e-10",
        ". as $n | [range(1;1000001)] | [(.[$n:] | length), (.[:$n] | length)]",
        "[1,1000000]\n",
    ),
    (
        "-5e-11",
        ". as $n | [range(1;1000001)] | [(.[$n:] | length), (.[:$n] | length)]",
        "[0,1000000]\n",
    ),
];

#[test]
fn a_negative_fractional_slice_bound_is_added_to_the_length_before_it_is_rounded_6984() -> Result<()>
{
    for &(input, filter, want) in ROWS {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(input))?;
        assert_eq!(
            (stdout.as_str(), code),
            (want, 0),
            "{input} | {filter}: {stderr}"
        );
    }
    Ok(())
}
