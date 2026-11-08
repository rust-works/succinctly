//! #6692: a loop that reads the tail of an array the program built (`.[$i:][0]`, `.[$i:][:2]`,
//! `.[$i:] | length`) was quadratic: `eval.rs`'s pipes built the slice, the whole tail, and then read
//! one element or two of it, where the generic evaluator already answered `.[a:b][k]` from the slice's
//! resolved range (#4195, #4288). Both pipe drivers of `eval.rs` take that fused head now, and a
//! second literal slice (`.[a:b][c:d]`) composes onto the first's range, for both evaluators.
//! Every expected string was captured from `/usr/bin/jq` 1.7.1.

use std::time::{Duration, Instant};

use anyhow::Result;

use crate::{run_jq_full, run_jq_stdin};

#[test]
fn a_fused_slice_read_and_a_slice_of_a_slice_answer_as_jq_does_6692() -> Result<()> {
    // (input, program, stdout, exit status); the loops run the programs through `eval.rs`'s pipes.
    for (input, program, stdout, status) in [
        ("[0,1,2,3,4,5,6,7,8,9]", ".[2:][:3]", "[2,3,4]\n", 0),
        ("[0,1,2,3,4,5,6,7,8,9]", ".[-3:][:2]", "[7,8]\n", 0),
        ("[0,1,2,3,4,5,6,7,8,9]", ".[:5][-2:]", "[3,4]\n", 0),
        ("[0,1,2,3,4,5,6,7,8,9]", ".[3:7][1:3]", "[4,5]\n", 0),
        ("[0,1,2,3,4,5,6,7,8,9]", ".[7:3][:1]", "[]\n", 0),
        ("[0,1,2,3,4,5,6,7,8,9]", ".[100:][:2]", "[]\n", 0),
        ("[0,1,2,3,4,5,6,7,8,9]", ".[1.5:][:2]", "[1,2]\n", 0),
        ("[0,1,2,3,4,5,6,7,8,9]", ".[2:][5:]", "[7,8,9]\n", 0),
        ("[0,1,2,3,4,5,6,7,8,9]", ".[2:][:0]", "[]\n", 0),
        ("[0,1,2,3,4,5,6,7,8,9]", ".[-100:][:2]", "[0,1]\n", 0),
        ("[0,1,2,3,4,5,6,7,8,9]", ".[:100][-3:]", "[7,8,9]\n", 0),
        (
            "[0,1,2,3,4,5,6,7,8,9]",
            "[range(1) as $i | .[$i+2:][:3]]",
            "[[2,3,4]]\n",
            0,
        ),
        (
            "[0,1,2,3,4,5,6,7,8,9]",
            "[range(0;10;3) as $i | .[$i:][:2]]",
            "[[0,1],[3,4],[6,7],[9]]\n",
            0,
        ),
        (
            "[0,1,2,3,4,5,6,7,8,9]",
            "[range(0;10;3) as $i | .[$i:][0]]",
            "[0,3,6,9]\n",
            0,
        ),
        (
            "[0,1,2,3,4,5,6,7,8,9]",
            "[range(0;10;3) as $i | .[$i:][-1]]",
            "[9,9,9,9]\n",
            0,
        ),
        (
            "[0,1,2,3,4,5,6,7,8,9]",
            "[range(0;10;4) as $i | .[$i:] | length]",
            "[10,6,2]\n",
            0,
        ),
        ("[0,1,2,3,4,5,6,7,8,9]", ".[2:][0]", "2\n", 0),
        ("[0,1,2,3,4,5,6,7,8,9]", ".[-3:][-1]", "9\n", 0),
        ("[0,1,2,3,4,5,6,7,8,9]", ".[2:] | length", "8\n", 0),
        ("[0,1,2,3,4,5,6,7,8,9]", ".[100:][0]", "null\n", 0),
        ("[0,1,2,3,4,5,6,7,8,9]", ".[:0] | first", "null\n", 0),
        ("[0,1,2,3,4,5,6,7,8,9]", ".[2:] | last", "9\n", 0),
        ("[0,1,2,3,4,5,6,7,8,9]", "[first(.[3:][0])]", "[3]\n", 0),
        (
            "[0,1,2,3,4,5,6,7,8,9]",
            "[range(0;10;5) as $i | select(.[$i:][0] >= 5) | $i]",
            "[5]\n",
            0,
        ),
        (
            "[0,1,2,3,4,5,6,7,8,9]",
            "[range(0;10;5) as $i | limit(1; .[$i:][:2])]",
            "[[0,1],[5,6]]\n",
            0,
        ),
        (
            "[0,1,2,3,4,5,6,7,8,9]",
            ".[2:][1:][1:]",
            "[4,5,6,7,8,9]\n",
            0,
        ),
        ("[0,1,2,3,4,5,6,7,8,9]", ".[1:3][:5]", "[1,2]\n", 0),
        ("\"hello\"", ".[1:][:2]", "\"el\"\n", 0),
        ("null", ".[1:][:2]", "null\n", 0),
        ("[]", ".[1:][:2]", "[]\n", 0),
        ("[]", ".[1:][0]", "null\n", 0),
        ("{\"a\":1}", ".[1:][:2]", "", 5),
    ] {
        let (out, code) = run_jq_stdin(program, input, &["-c"])?;
        assert_eq!(out, stdout, "{input} | {program}");
        assert_eq!(code, status, "{input} | {program}");
    }
    Ok(())
}

/// jq parses the whole input first, so a malformed element anywhere rejects the document. A fused
/// slice read does not validate the rest of the range (`docs/compliance/jq/limitations.md`, #4195):
/// the element a read names is converted, the others are not, and since #6692 a second slice is
/// composed onto the first's range, so an element of the first slice that the second drops is not
/// converted either. A bare slice, and a read that names the malformed element, still raise.
#[test]
fn a_malformed_element_the_composed_slice_drops_is_not_read_6692() -> Result<()> {
    for (input, program, stdout, status) in [
        ("[1,{\"a\":},3]", ".[:2][:1]", "[1]\n", 0),
        ("[1,{\"a\":},3]", ".[1:][1:]", "[3]\n", 0),
        ("[1,tru,3]", ".[:2][:1]", "[1]\n", 0),
        ("[1,tru,3]", ".[1:] | length", "2\n", 0),
        ("[1,tru,3]", ".[:2][:2]", "", 5),
        ("[1,tru,3]", ".[1:][0]", "", 5),
        ("[1,tru,3]", ".[:2]", "", 5),
    ] {
        let (out, code) = run_jq_stdin(program, input, &["-c"])?;
        assert_eq!(out, stdout, "{input} | {program}");
        assert_eq!(code, status, "{input} | {program}");
    }
    Ok(())
}

/// The reads and the slice of a slice in the shapes of a loop over a built array, one per pipe
/// driver: a plain pipe, a generator's (`first`, `select`, `limit`) and one inside a `try`. The
/// array is 30,000 long, so a loop that builds each tail is over 4e8 element conversions, which
/// no machine does within the bound, where the fused read is a few thousand.
#[test]
fn a_loop_over_a_moving_tail_is_not_quadratic_6692() -> Result<()> {
    let n = 30_000;
    for body in [
        ".[$i:][0]",
        ".[$i:][-1]",
        ".[$i:][:2]",
        ".[$i:] | length",
        "first(.[$i:][0])",
        "first(.[$i:][:2] | length)",
        "select(.[$i:][0] >= 0) | 1",
        "limit(1; .[$i:][:2])",
        "try (.[$i:][:2]) catch 0",
    ] {
        let program = format!("[range({n})] | [range(0; {n}; 2) as $i | {body}] | length");
        let started = Instant::now();
        let (out, err, code) = run_jq_full(&["-n", "-c", &program], None)?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            ("15000\n", "", 0),
            "{body}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "{body}: {:?}",
            started.elapsed()
        );
    }
    Ok(())
}
