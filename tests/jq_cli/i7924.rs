//! #7924: a number literal past decNumber's exponent range compares as jq's decNumber holds it,
//! not as written.
//!
//! jq parses a literal with `decNumberFromString` in a context whose `Etiny` is -1147483646 and
//! whose `Emax` is 999999999, rounding half-up: `1e-1999999999` is zero and `12e-1147483647` is
//! `1e-1147483646`, which is what both print, and `1e1000000000` is an infinity. `jvp_number_cmp`
//! compares two literals with `decNumberCompare` on those rounded values. `succinctly jq` printed
//! the rounded value but compared the digits as written, so `1e-1999999999 == 0` was `false`, and
//! negated such a literal to `-0E-1147483646`, which neither release prints.
//!
//! Every row was captured from jq 1.7.1 (`/usr/bin/jq -c`) and jq 1.8.2 (`/opt/homebrew/bin/jq`),
//! which agree, except the rows that negate: there `succinctly jq` follows jq 1.8.2 (#7453), whose
//! `-` keeps a literal, where jq 1.7.1 computes the double `-0`.

use anyhow::Result;

use crate::{run_jq_null, run_jq_stdin};

/// `(program, stdout)`, run as `jq -nc PROGRAM`.
const NULL_INPUT_ROWS: &[(&str, &str)] = &[
    (
        "[1e-1147483646, 1e-1147483647, 12e-1147483647, 1e-1999999999]",
        "[1E-1147483646,0E-1147483646,1E-1147483646,0E-1147483646]\n",
    ),
    (
        "[(1e-1999999999 == 0), (1e-1999999999 > 0), (12e-1147483647 == 1e-1147483646), \
         ([1e-1999999999, 0] | unique), ([1e-1999999999] | min == 0), (1e-1999999999 * 1)]",
        "[true,false,true,[0E-1147483646],true,0]\n",
    ),
    (
        "[1e-1999999999, 0, 5e-1147483647, 1e-1147483646] \
         | [sort, unique, (group_by(.) | map(length))]",
        "[[0E-1147483646,0,1E-1147483646,1E-1147483646],[0E-1147483646,1E-1147483646],[2,2]]\n",
    ),
    (
        "[1e1000000000 == 2e1000000000, 1e999999999 < 1e1000000000, \
         ([1e1000000000, 1e1999999999] | unique | length)]",
        "[true,true,1]\n",
    ),
    // jq 1.8.2's answer; jq 1.7.1's is `[-0,-0,-0]`.
    (
        "[-1e-1999999999, (1e-1999999999 | -.), -0e-999999999999999999999999999999]",
        "[0E-1147483646,0E-1147483646,0E-1147483646]\n",
    ),
];

#[test]
fn literal_past_the_exponent_range_compares_rounded_7924() -> Result<()> {
    for (program, expected) in NULL_INPUT_ROWS {
        let (out, code) = run_jq_null(program, &["-c"])?;
        assert_eq!((out.as_str(), code), (*expected, 0), "{program}");
    }
    Ok(())
}

#[test]
fn document_and_argjson_numbers_compare_rounded_7924() -> Result<()> {
    let (out, code) = run_jq_stdin(
        "[.[0] == .[1], .[2] == .[3], (unique | length)]",
        "[1e-1999999999, 0, 1e1000000000, 2e1000000000]",
        &["-c"],
    )?;
    assert_eq!((out.as_str(), code), ("[true,true,2]\n", 0));
    // A spelled infinity (#7469) is the infinity a literal past `Emax` becomes.
    let (out, code) = run_jq_stdin(
        "[.[0] == .[1], .[2] == .[3], .[0] > .[4], (unique | length)]",
        "[Infinity, 1e1000000000, -Infinity, -2e1000000000, 1e999999999]",
        &["-c"],
    )?;
    assert_eq!((out.as_str(), code), ("[true,true,true,3]\n", 0));
    // jq 1.8.2's answer; jq 1.7.1's is `[0E-1147483646,-0,-0,true]`.
    let (out, code) = run_jq_null(
        "[$a, -$a, ($a | -.), $a == 0]",
        &["-c", "--argjson", "a", "1e-1999999999"],
    )?;
    assert_eq!(
        (out.as_str(), code),
        ("[0E-1147483646,0E-1147483646,0E-1147483646,true]\n", 0)
    );
    Ok(())
}
