//! #7495: an assignment whose array index is a float-represented whole number (`1e4`, `3.0`,
//! `$i * 1.0`, an element of `[range(1e4)]`) writes the slot its integer spelling names, on the
//! same in-place route an integer index takes.
//!
//! Before the fix such a key declined the fold's in-place write, and the evaluator's route rebuilt
//! the accumulator on every step, so `reduce range(1e4) as $i ([]; .[$i] = 1)` was quadratic
//! where `reduce range(10000) ...` was linear. The answers were already jq's; these rows pin them
//! so the fast path cannot change one. The scaling itself is pinned by
//! `owned_assign_step_takes_a_whole_float_index_in_place_7495` in `src/jq/eval.rs`, which asserts
//! that each step is answered in place rather than timing anything.
//!
//! Every expected output below was captured from `/usr/bin/jq` (jq 1.7.1); jq 1.8.2 prints the
//! same for every row here. (They differ only on keys this file leaves out: jq 1.7.1 crashes on
//! an index of `1e300`, `infinite` or `1e18`, where jq 1.8.2 raises `Array index too large`, and
//! jq 1.8.2 adds the key to `Cannot index object with number (1.0)`.)

use anyhow::Result;

use crate::run_jq_full;

/// (filter, `jq -nc` stdout, exit code). An error row's stdout is empty and its stderr is
/// checked separately in [`ERRORS`].
const ROWS: &[(&str, &str)] = &[
    // Whole floats name the integer's slot.
    ("[1,2,3] | .[1.0] = 9", "[1,9,3]\n"),
    ("[1,2,3] | .[-0.0] = 9", "[9,2,3]\n"),
    ("[1,2,3] | .[-1.0] = 9", "[1,2,9]\n"),
    ("[1,2,3] | .[5.0] = 9", "[1,2,3,null,null,9]\n"),
    ("[1,2,3] | .[1.0] |= .+10", "[1,12,3]\n"),
    ("[1,2,3] | .[1.0] += 10", "[1,12,3]\n"),
    ("[1,2,3] | .[-0.0] += 10", "[11,2,3]\n"),
    ("[1,null,3] | .[1.0] //= 10", "[1,10,3]\n"),
    ("null | .[2.0] = 1", "[null,null,1]\n"),
    ("null | .[0.0] = 1", "[1]\n"),
    // Fractional keys truncate toward zero.
    ("[1,2,3] | .[1.5] = 9", "[1,9,3]\n"),
    ("[1,2,3] | .[1.7] = 9", "[1,9,3]\n"),
    ("[1,2,3] | .[-1.7] = 9", "[1,2,9]\n"),
    ("[1,2,3] | .[1.5] |= .+10", "[1,12,3]\n"),
    ("[1,2,3] | .[1.5] += 10", "[1,12,3]\n"),
    ("[1,null,3] | .[1.5] //= 10", "[1,10,3]\n"),
    ("null | .[2.5] = 1", "[null,null,1]\n"),
    // Reads, `getpath`, `setpath`, `path` and `del` with the same keys.
    ("[1,2,3] | .[1.0]", "2\n"),
    ("[1,2,3] | .[1.5]", "2\n"),
    ("[1,2,3] | .[-0.0]", "1\n"),
    ("[1,2,3] | .[nan]", "null\n"),
    ("[1,2,3] | getpath([1.0])", "2\n"),
    ("[1,2,3] | getpath([1.5])", "2\n"),
    ("[1,2,3] | setpath([1.0]; 9)", "[1,9,3]\n"),
    ("[1,2,3] | setpath([1.5]; 9)", "[1,9,3]\n"),
    ("[1,2,3] | path(.[1.0])", "[1.0]\n"),
    ("[1,2,3] | path(.[1.5])", "[1.5]\n"),
    ("[1,2,3] | del(.[1.0])", "[1,3]\n"),
    ("[1,2,3] | del(.[1.5])", "[1,3]\n"),
    // The fold shapes of the issue: a whole float key on the in-place route.
    (
        "reduce (0.0, 1.0, 2.0) as $i ([]; .[$i] = $i)",
        "[0.0,1.0,2.0]\n",
    ),
    (
        "reduce (0.5, 1.5, 2.5) as $i ([]; .[$i] = $i)",
        "[0.5,1.5,2.5]\n",
    ),
    (
        "reduce (2.0, 0.0) as $i (null; .[$i] = $i)",
        "[0.0,null,2.0]\n",
    ),
    (
        "[foreach (0.0, 1.0, 1.0) as $i ([]; .[$i] += 1)]",
        "[[1],[1,1],[1,2]]\n",
    ),
    (
        "reduce range(1e1) as $i ([]; .[$i] = 1)",
        "[1,1,1,1,1,1,1,1,1,1]\n",
    ),
    (
        "reduce range(3) as $i ([1,2,3]; .[$i*1.0] += 1)",
        "[2,3,4]\n",
    ),
    ("reduce (3.0) as $i ([1,2]; .[$i] = 1)", "[1,2,null,1]\n"),
    ("reduce (-0.0) as $i ([1,2]; .[$i] = 7)", "[7,2]\n"),
    ("reduce (0.0) as $i ([[1]]; .[$i][$i] = 7)", "[[7]]\n"),
    (
        "reduce (1.0) as $i ({\"a\":[1,2]}; .a[$i] = 7)",
        "{\"a\":[1,7]}\n",
    ),
    // The issue's own programs, at its size.
    (
        "reduce range(1e4) as $i ([]; .[$i] = 1) | length",
        "10000\n",
    ),
    (
        "reduce range(10000) as $i ([]; .[$i*1.0] = 1) | length",
        "10000\n",
    ),
    ("reduce range(1e4) as $i ([]; .[$i] += 1) | add", "10000\n"),
    ("reduce range(1e4) as $i ([]; .[$i] //= 2) | add", "20000\n"),
    ("reduce range(1e4) as $i ([]; .[$i] |= 3) | add", "30000\n"),
    (
        "[range(1e4)] | reduce range(1e4) as $i (.; .[$i] = 2) | add",
        "20000\n",
    ),
    (
        "[range(10000) | . + 0.0] | reduce .[] as $i (.; .[$i] += 1) | add",
        "50005000\n",
    ),
    (
        "[foreach range(1e4) as $i ([]; .[$i] = $i; length)] | last",
        "10000\n",
    ),
];

/// (filter, jq 1.7.1's stderr). Each exits 5 with no stdout.
const ERRORS: &[(&str, &str)] = &[
    (
        "[1,2,3] | .[nan] = 9",
        "jq: error (at <unknown>): Cannot set array element at NaN index\n",
    ),
    (
        "[1,2,3] | .[-infinite] = 9",
        "jq: error (at <unknown>): Out of bounds negative array index\n",
    ),
    (
        "reduce range(1;4) as $i ([]; .[-$i*1.0] = 1)",
        "jq: error (at <unknown>): Out of bounds negative array index\n",
    ),
    (
        "{} | .[1.0] = 1",
        "jq: error (at <unknown>): Cannot index object with number\n",
    ),
    (
        "{\"a\":1} | .[0.0] += 1",
        "jq: error (at <unknown>): Cannot index object with number\n",
    ),
    (
        "reduce (1.0) as $i ({}; .[$i] = 1)",
        "jq: error (at <unknown>): Cannot index object with number\n",
    ),
    (
        "reduce (1.0) as $i (\"x\"; .[$i] = 1)",
        "jq: error (at <unknown>): Cannot index string with number\n",
    ),
];

#[test]
fn whole_float_index_writes_match_jq_7495() -> Result<()> {
    for (filter, want) in ROWS {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!((stdout.as_str(), code), (*want, 0), "`{filter}`: {stderr}");
    }
    Ok(())
}

#[test]
fn float_index_errors_match_jq_7495() -> Result<()> {
    for (filter, want) in ERRORS {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            ("", *want, 5),
            "`{filter}`"
        );
    }
    Ok(())
}
