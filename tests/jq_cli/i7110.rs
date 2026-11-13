//! #7110: arithmetic over a count and number literals (`length - 5`) is answered from the owned
//! container's length, so `$a[$a | length - 5:]` no longer writes `$a` out as text and indexes it
//! again for every bound it evaluates. Every row is `jq -nc PROGRAM` against what jq 1.7.1
//! (`/usr/bin/jq`) prints, with exit status 0; the rows that raise, shadow `length` or are not a
//! container are the ones a door that read the stage too loosely would change.

use anyhow::Result;

use crate::run_jq_full;

#[rustfmt::skip]
const ROWS: &[(&str, &str)] = &[
    (
        "[range(10)] | . as $a | $a[$a|length - 5:]",
        "[5,6,7,8,9]\n",
    ),
    (
        "[range(10)] | . as $a | $a[:$a|length - 5]",
        "[0,1,2,3,4]\n",
    ),
    (
        "[range(10)] | . as $a | [range(3) | $a[$a|length - 2 * 3:]] | map(length)",
        "[6,6,6]\n",
    ),
    (
        "[range(10)] | . as $a | [range(2) | $a | length - 1]",
        "[9,9]\n",
    ),
    (
        "[range(10)] | . as $a | [range(2) | $a | 100 - length]",
        "[90,90]\n",
    ),
    (
        "[range(10)] | . as $a | [range(2) | $a | (length - 1) * (length + 1)]",
        "[99,99]\n",
    ),
    (
        "{\"a\":1,\"b\":2} | . as $a | [range(2) | $a | length * 2 - 1]",
        "[3,3]\n",
    ),
    (
        "[range(10)] | . as $a | [range(2) | $a | length / 4]",
        "[2.5,2.5]\n",
    ),
    (
        "[range(10)] | . as $a | [range(2) | $a | length % 4]",
        "[2,2]\n",
    ),
    (
        "[range(10)] | . as $a | [range(2) | $a | length + 0.5]",
        "[10.5,10.5]\n",
    ),
    (
        "[range(10)] | . as $a | [range(2) | $a | length * 1e1000]",
        "[1.7976931348623157e+308,1.7976931348623157e+308]\n",
    ),
    (
        "[range(10)] | . as $a | [range(2) | $a | try (length / 0) catch .]",
        "[\"number (10) and number (0) cannot be divided because the divisor is zero\",\"number (10) and number (0) cannot be divided because the divisor is zero\"]\n",
    ),
    (
        "[range(10)] | . as $a | [range(2) | $a | try (length % 0) catch .]",
        "[\"number (10) and number (0) cannot be divided (remainder) because the divisor is zero\",\"number (10) and number (0) cannot be divided (remainder) because the divisor is zero\"]\n",
    ),
    (
        "[range(10)] | . as $a | [range(2) | $a | try (length - \"x\") catch .]",
        "[\"number (10) and string (\\\"x\\\") cannot be subtracted\",\"number (10) and string (\\\"x\\\") cannot be subtracted\"]\n",
    ),
    (
        "[range(10)] | . as $a | [range(2) | $a | (length + \"x\")? // \"no\"]",
        "[\"no\",\"no\"]\n",
    ),
    (
        "\"héllo\" | ascii_downcase | . as $a | [range(2) | $a | length - 1]",
        "[4,4]\n",
    ),
    (
        "null | . as $a | [range(2) | $a | length - 1]",
        "[-1,-1]\n",
    ),
    (
        "5 | . as $a | [range(2) | $a | length - 1]",
        "[4,4]\n",
    ),
    (
        "def length: 7; [range(10)] | . as $a | [range(2) | $a | length - 1]",
        "[6,6]\n",
    ),
    (
        "[range(10)] | . as $a | [range(2) | $a | length - length]",
        "[0,0]\n",
    ),
];

#[test]
fn count_arithmetic_matches_jq_7110() -> Result<()> {
    for &(program, expected) in ROWS {
        let (stdout, stderr, status) = run_jq_full(&["-nc", program], None)?;
        assert_eq!(stdout, expected, "stdout of {program}");
        assert_eq!(status, 0, "exit status of {program}: {stderr}");
    }
    Ok(())
}
