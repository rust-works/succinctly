//! #7731: an all-integer `range` stepped in exact `i64` arithmetic at every magnitude, where jq
//! steps in doubles (`range/2` is the `RANGE` opcode, `range/3` is
//! `$from | while(. < $upto; . + $by)`). Within ±2^53 the two walks agree value for value; past it
//! jq's sums round, its comparisons see the rounded doubles, and it prints the double. Every row
//! is captured from `/usr/bin/jq` 1.7.1 (macOS; the Linux build agrees), and
//! `/opt/homebrew/bin/jq` 1.8.2 agrees on every row but the one marked.

use anyhow::Result;

use crate::run_jq_full;

/// (input, or `None` for `-n`; filter; stdout).
const ROWS: &[(Option<&str>, &str, &str)] = &[
    // `range/2`: the sum after `-9007199254740994` rounds to `-9007199254740992`.
    (
        None,
        "[range(-9007199254740994; -9007199254740990)]",
        "[-9007199254740994,-9007199254740992,-9007199254740991]\n",
    ),
    // Both bounds are the double 2^63.
    (
        None,
        "[range(9223372036854775806; 9223372036854775807)]",
        "[]\n",
    ),
    (
        None,
        "[range(1700000000000000000; 1700000000000000010)]",
        "[]\n",
    ),
    // `+ 1` rounds back to 2^53, so jq repeats it (and never ends without the `limit`).
    (
        None,
        "[limit(5; range(9007199254740990; 9007199254740994))]",
        "[9007199254740990,9007199254740991,9007199254740992,9007199254740992,9007199254740992]\n",
    ),
    // The first value is `$from` itself, as written; the rest are doubles (#6064).
    (
        None,
        "[limit(3; range(9007199254740993; 9007199254740996))] | ., map(tojson)",
        "[9007199254740993,9007199254740992,9007199254740992]\n[\"9007199254740993\",\"9007199254740992\",\"9007199254740992\"]\n",
    ),
    (
        Some("[9007199254740993,9007199254741000]"),
        "[limit(5; range(.[0]; .[1]))]",
        "[9007199254740993,9007199254740992,9007199254740992,9007199254740992,9007199254740992]\n",
    ),
    // jq 1.8.2; jq 1.7.1 is `[-9223372036854776000,-9223372036854776000,-9223372036854776000]`,
    // because it negates the literal to a double, where 1.8 negates it to a literal
    // (jqlang/jq#3242), which `succinctly jq` follows (#7453).
    (
        None,
        "[limit(3; range(-9223372036854775808; 0))]",
        "[-9223372036854775808,-9223372036854776000,-9223372036854776000]\n",
    ),
    // `range/3`.
    (
        None,
        "[range(-9007199254740994; -9007199254740990; 1)]",
        "[-9007199254740994,-9007199254740992,-9007199254740991]\n",
    ),
    (
        None,
        "[range(0; -9007199254740994; -9007199254740993)]",
        "[0,-9007199254740992]\n",
    ),
    (
        None,
        "[range(1700000000000000000; 1700000000000000010; 1)]",
        "[1700000000000000000]\n",
    ),
    // jq's first `$from < $upto` compares the two literals as decimals, so this is not empty
    // as `range/2` over the same bounds is; the next value is the double 2^63.
    (
        None,
        "[range(9223372036854775806; 9223372036854775807; 1)]",
        "[9223372036854775806]\n",
    ),
    (
        None,
        "[range(9223372036854775806; 9223372036854775808; 1)]",
        "[9223372036854775806]\n",
    ),
    (
        Some("[9223372036854775806,9223372036854775807]"),
        "[range(.[0]; .[1]; 1)], [range(.[0]; .[1])]",
        "[9223372036854775806]\n[]\n",
    ),
    (
        None,
        "[range(9223372036854774000;9223372036854775807;1000000000000000)]",
        "[9223372036854774000]\n",
    ),
    (
        Some("[0,-9223372036854775808,-4611686018427388003]"),
        "[range(.[0]; .[1]; .[2])]",
        "[0,-4611686018427388000]\n",
    ),
    (
        Some("[-9223372036854775758,-9223372036854775808,-100]"),
        "[range(.[0];.[1];.[2])]",
        "[-9223372036854775758]\n",
    ),
    // A step past 2^53: it is the double 9007199254740992.
    (
        None,
        "[range(0; 36028797018963968; 9007199254740993)]",
        "[0,9007199254740992,18014398509481984,27021597764222976]\n",
    ),
    (
        None,
        "[limit(4; range(0; infinite; 9007199254740993))]",
        "[0,9007199254740992,18014398509481984,27021597764222976]\n",
    ),
    (
        None,
        "[limit(4; range(0.5; 1e300; 9007199254740993))]",
        "[0.5,9007199254740992,18014398509481984,27021597764222976]\n",
    ),
    // Within ±2^53 the `i64` walk is jq's walk.
    (
        None,
        "[range(9007199254740990; 9007199254740992)]",
        "[9007199254740990,9007199254740991]\n",
    ),
    (
        None,
        "[range(-9007199254740992; -9007199254740990)]",
        "[-9007199254740992,-9007199254740991]\n",
    ),
    (
        None,
        "[range(-1; 9007199254740994; 4503599627370496)]",
        "[-1,4503599627370495,9007199254740991]\n",
    ),
    (
        None,
        "[range(4; -9007199254740994; -4503599627370497)]",
        "[4,-4503599627370493,-9007199254740990]\n",
    ),
    (None, "[range(0; 1; 0.3)]", "[0,0.3,0.6,0.8999999999999999]\n"),
];

#[test]
fn test_range_past_2_53_steps_in_doubles_7731() -> Result<()> {
    for &(input, filter, want) in ROWS {
        let args: &[&str] = if input.is_some() {
            &["-c", filter]
        } else {
            &["-nc", filter]
        };
        let got = run_jq_full(args, input)?;
        assert_eq!(
            got,
            (want.to_string(), String::new(), 0),
            "{filter} over {input:?}"
        );
    }
    Ok(())
}
