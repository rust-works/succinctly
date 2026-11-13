//! #7109: a fold whose UPDATE is a slice of the state plus a closed operand, `.[1:] + [$i]` (a queue
//! kept by dropping its head and appending), is a step on the owned state, as `.[1:] | . + [$i]` is
//! (#6575): the cut is in place (#6659) and the combine is the plain arithmetic of the cut and the
//! operand. Through the bridge every step wrote the whole accumulator out as JSON and indexed it
//! again, which made the fold quadratic (41 s at n = 20,000, where jq takes 0.05 s).
//!
//! Every row was captured from jq 1.7.1 (`/usr/bin/jq -c -n`): stdout, stderr and the exit status.

use std::time::{Duration, Instant};

use anyhow::Result;

use crate::{run_jq_full, run_jq_null};

/// `(program, stdout, stderr, exit status)`.
const ROWS: &[(&str, &str, &str, i32)] = &[
    (
        "[range(5)] | reduce range(3) as $i (.; .[1:] + [$i])",
        "[3,4,0,1,2]\n",
        "",
        0,
    ),
    (
        "[range(6)] | reduce range(3) as $i (.; .[1:3] + [$i])",
        "[0,1,2]\n",
        "",
        0,
    ),
    (
        "[range(6)] | reduce range(3) as $i (.; .[:-1] + [$i])",
        "[0,1,2,3,4,2]\n",
        "",
        0,
    ),
    (
        "[range(6)] | reduce range(3) as $i (.; .[-2:] + [$i])",
        "[0,1,2]\n",
        "",
        0,
    ),
    (
        "[range(6)] | reduce range(3) as $i (.; .[2:] + [$i, $i])",
        "[0,0,1,1,2,2]\n",
        "",
        0,
    ),
    (
        "[range(6)] | reduce range(3) as $i (.; .[1:] - [$i])",
        "[3,4,5]\n",
        "",
        0,
    ),
    (
        "[range(6)] | reduce range(3) as $i (.; .[1:] + [[$i]])",
        "[3,4,5,[0],[1],[2]]\n",
        "",
        0,
    ),
    (
        "[range(6)] | reduce range(3) as $i (.; (.[1:]) + [$i])",
        "[3,4,5,0,1,2]\n",
        "",
        0,
    ),
    (
        "\"abcdef\" | reduce range(3) as $i (.; .[1:] + \"x\")",
        "\"defxxx\"\n",
        "",
        0,
    ),
    (
        "null | reduce range(2) as $i (.; .[1:] + [$i])",
        "[1]\n",
        "",
        0,
    ),
    (
        "{} | reduce range(1) as $i (.; .[1:] + [$i])",
        "",
        "jq: error (at <unknown>): Cannot index object with object\n",
        5,
    ),
    (
        "5 | reduce range(1) as $i (.; .[1:] + [$i])",
        "",
        "jq: error (at <unknown>): Cannot index number with object\n",
        5,
    ),
    (
        "[range(3)] | reduce range(2) as $i (.; .[1:] + 1)",
        "",
        "jq: error (at <unknown>): array ([1,2]) and number (1) cannot be added\n",
        5,
    ),
    (
        "[range(3)] | reduce range(2) as $i (.; .[1:] + null)",
        "[2]\n",
        "",
        0,
    ),
    (
        "[range(5)] | [foreach range(3) as $i (.; .[1:] + [$i])]",
        "[[1,2,3,4,0],[2,3,4,0,1],[3,4,0,1,2]]\n",
        "",
        0,
    ),
    (
        "[range(5)] | [foreach range(3) as $i (.; .[1:] + [$i]; length)]",
        "[5,5,5]\n",
        "",
        0,
    ),
    (
        "[range(5)] | until(length > 7; .[1:] + [9,9])",
        "[3,4,9,9,9,9,9,9]\n",
        "",
        0,
    ),
    (
        "[range(5)] | reduce range(3) as $i (.; .[1:] + [.[0]])",
        "[3,4,0,1,2]\n",
        "",
        0,
    ),
    (
        "[range(5)] | reduce range(3) as $i (.; .[1:] + [length])",
        "[3,4,5,5,5]\n",
        "",
        0,
    ),
    (
        "[range(5)] as $a | [reduce range(3) as $i ($a; .[1:] + [$i]), $a]",
        "[[3,4,0,1,2],[0,1,2,3,4]]\n",
        "",
        0,
    ),
    (
        "[range(5)] | . as $a | reduce range(3) as $i (.; .[1:] + [$i]) | [., $a]",
        "[[3,4,0,1,2],[0,1,2,3,4]]\n",
        "",
        0,
    ),
    (
        "reduce range(3) as $i ([range(5)]; (.[1:] + [$i])?)",
        "[3,4,0,1,2]\n",
        "",
        0,
    ),
    ("reduce range(3) as $i (1; (.[1:] + [$i])?)", "[2]\n", "", 0),
    (
        "[range(5)] | reduce range(3) as $i (.; try (.[1:] + [$i]) catch .)",
        "[3,4,0,1,2]\n",
        "",
        0,
    ),
    (
        "[range(5)] | reduce (1,2) as $i (.; .[1:] + [$i, -$i])",
        "[2,3,4,1,-1,2,-2]\n",
        "",
        0,
    ),
    (
        "[range(5)] | reduce range(3) as $i (.; .[1.5:] + [$i])",
        "[3,4,0,1,2]\n",
        "",
        0,
    ),
    (
        "[range(5)] | reduce range(3) as $i (.; .[:null] + [$i])",
        "[0,1,2,3,4,0,1,2]\n",
        "",
        0,
    ),
    (
        "[range(5)] | reduce range(7) as $i (.; .[10:] + [$i])",
        "[6]\n",
        "",
        0,
    ),
    (
        "[range(5)] | reduce range(3) as $i (.; .[1:] * 2)",
        "",
        "jq: error (at <unknown>): array ([1,2,3,4]) and number (2) cannot be multiplied\n",
        5,
    ),
    (
        "[range(4)] | reduce range(2) as $i (.; .[1:] + [$i] + [9])",
        "[2,3,0,9,1,9]\n",
        "",
        0,
    ),
];

#[test]
fn a_slice_plus_a_closed_operand_as_a_fold_update_answers_as_jq_does_7109() -> Result<()> {
    for &(program, stdout, stderr, code) in ROWS {
        let (out, err, status) = run_jq_full(&["-c", "-n", program], None)?;
        assert_eq!(
            (out.as_str(), err.as_str(), status),
            (stdout, stderr, code),
            "{program}"
        );
    }
    Ok(())
}

/// The row of the issue, at a size where a per-step re-index shows as seconds (41 s before the step,
/// on a laptop) and the step as milliseconds: the bound is far from either, so the pin is the shape
/// of the cost and not a timing.
#[test]
fn a_queue_kept_by_slice_plus_append_is_linear_7109() -> Result<()> {
    let start = Instant::now();
    let (out, code) = run_jq_null(
        "[range(20000)] | reduce range(20000) as $i (.; .[1:] + [$i]) | [length, .[0]]",
        &["-c"],
    )?;
    let took = start.elapsed();
    assert_eq!((out.trim(), code), ("[20000,0]", 0));
    assert!(took < Duration::from_secs(15), "{took:?}");
    Ok(())
}
