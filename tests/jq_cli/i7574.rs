//! #7574: a write past the end of a slice pads with `null`, recorded in
//! `docs/compliance/jq/limitations.md` ("A write past the end of a slice pads with `null`; jq can
//! show the elements the slice dropped").
//!
//! jq's slice is a view over the parent's buffer, and a write past the view's end reuses that buffer
//! in place when nothing else holds it and the index fits its capacity, so the parent's elements
//! after the slice come back (`[1,2,3,4,5,6] | .[:1] | .[3] = 9` is `[1,2,3,9]` in jq 1.7.1 and
//! 1.8.2). Whether it does depends on the buffer's capacity and on jq's reference counts, not on the
//! value, and the output carries elements the program cut away, so `succinctly jq` pads everywhere.
//! The comment on each divergent row gives jq's answer; the other rows are jq's answer, captured
//! from `/usr/bin/jq` 1.7.1 (`/opt/homebrew/bin/jq` 1.8.2 agrees on every row).

use anyhow::Result;

use crate::run_jq_full;

const SIX: &str = "[1,2,3,4,5,6]\n";

/// (extra flags, input, filter, stdout): succinctly's answer, pinned.
const ROWS: &[(&[&str], &str, &str, &str)] = &[
    // The divergent rows. jq: `[1,2,3,9]` for the first five.
    (&[], SIX, ".[:1] | .[3] = 9", "[1,null,null,9]\n"),
    (&[], SIX, ".[:1] | setpath([3]; 9)", "[1,null,null,9]\n"),
    (&[], SIX, ".[:1] | .[3] |= 9", "[1,null,null,9]\n"),
    (&[], SIX, ".[:1] | .[3] += 9", "[1,null,null,9]\n"),
    (&[], SIX, ".[:1] | .[3] //= 9", "[1,null,null,9]\n"),
    // jq: `2`, `15`, `[1,2,3,9,8]`.
    (&[], SIX, ".[:1] | .[3] = 9 | .[1]", "null\n"),
    (&[], SIX, ".[:1] | .[3] = 9 | add", "10\n"),
    (
        &[],
        SIX,
        ".[:1] | .[3] = 9 | .[4] = 8",
        "[1,null,null,9,8]\n",
    ),
    // jq: `[2,3,4,9]`, `[3,4,5,9]`, `[5,6,9]`.
    (&[], SIX, ".[1:2] | .[3] = 9", "[2,null,null,9]\n"),
    (&[], SIX, ".[1:3] | .[1:] | .[3] = 9", "[3,null,null,9]\n"),
    (&[], SIX, ".[-2:-1] | .[2] = 9", "[5,null,9]\n"),
    // jq: `[1,2,null,9]` (the buffer holds two elements, its capacity is 16).
    (&[], "[1,2]\n", ".[:1] | .[3] = 9", "[1,null,null,9]\n"),
    // jq: `[1,2,3,9]` from `--slurp`, `input` and a built array; `{"a":[1,2,3,9]}`.
    (
        &["-s"],
        "1 2 3 4 5 6\n",
        ".[:1] | .[3] = 9",
        "[1,null,null,9]\n",
    ),
    (
        &["-n"],
        SIX,
        "input | .[:1] | .[3] = 9",
        "[1,null,null,9]\n",
    ),
    (
        &["-n"],
        "",
        "[range(6)] | .[:1] | .[3] = 9",
        "[0,null,null,9]\n",
    ),
    (
        &[],
        SIX,
        "{a: .[:1]} | .a[3] = 9",
        "{\"a\":[1,null,null,9]}\n",
    ),
    // jq: `[1,2,3,9]` through a dynamic target and a fold.
    (&[], SIX, ".[:1] | .[3 as $i | $i] = 9", "[1,null,null,9]\n"),
    (
        &[],
        SIX,
        ".[:1] | reduce 3 as $i (.; .[$i] = 9)",
        "[1,null,null,9]\n",
    ),
    // jq pads these itself: the parent is shared at the write, or the index is past the capacity.
    (
        &["-n"],
        "",
        "[1,2,3,4,5,6] | .[:1] | .[3] = 9",
        "[1,null,null,9]\n",
    ),
    (&[], SIX, ". as $x | .[:1] | .[3] = 9", "[1,null,null,9]\n"),
    (
        &[],
        SIX,
        "(.[:1] | .[3] = 9), .",
        "[1,null,null,9]\n[1,2,3,4,5,6]\n",
    ),
    (&[], SIX, ".[:1] | (.[3] = 9), .", "[1,null,null,9]\n[1]\n"),
    (&[], SIX, ".[:1] | .[(1,3)] = 9", "[1,9,null,9]\n"),
    (&[], SIX, ".[:1] | limit(1; .[3] = 9)", "[1,null,null,9]\n"),
    (
        &[],
        SIX,
        ".[:1] | .[16] = 9",
        "[1,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,9]\n",
    ),
    (
        &[],
        SIX,
        ".[:1] |= (.[3] = 9)",
        "[1,null,null,9,2,3,4,5,6]\n",
    ),
    (
        &[],
        "{\"a\":[1,2,3,4,5,6]}\n",
        ".a |= (.[:1] | .[3] = 9)",
        "{\"a\":[1,null,null,9]}\n",
    ),
    // Writes inside the view, `+`, `del` and slice assignment: jq builds a new array or writes
    // within the view, and agrees.
    (&[], SIX, ".[:1] | .[1] = 9", "[1,9]\n"),
    (&[], SIX, ".[:1] | . + [9]", "[1,9]\n"),
    (&[], SIX, ".[:1] | .[3:] = [9]", "[1,9]\n"),
    (&[], SIX, ".[:1] | del(.[3])", "[1]\n"),
    (
        &[],
        SIX,
        ".[:2] | del(.[0]) | .[3] = 9",
        "[2,null,null,9]\n",
    ),
    (&[], SIX, ".[:1] | .[1:] | .[3] = 9", "[null,null,null,9]\n"),
];

#[test]
fn test_a_write_past_a_slices_end_pads_with_null_7574() -> Result<()> {
    for &(flags, input, filter, want) in ROWS {
        let mut args = vec!["-c"];
        args.extend_from_slice(flags);
        args.push(filter);
        let got = run_jq_full(&args, Some(input))?;
        assert_eq!(
            got,
            (want.to_string(), String::new(), 0),
            "{flags:?} {filter} over {input:?}"
        );
    }
    Ok(())
}

/// `--stream` events, where the difference was found. jq's answer shows what its streaming parser
/// left in each event's buffer, which is not even the event's own value on the last line.
#[test]
fn test_a_write_past_a_stream_events_slice_pads_with_null_7574() -> Result<()> {
    // jq 1.7.1 and 1.8.2: `[[0],1,null,9]`, `[[1],null,null,9]`, `[[1],2,true,9]`.
    let got = run_jq_full(&["-c", "--stream", ".[:1] | .[3] = 9"], Some("[1,2]\n"))?;
    assert_eq!(
        got,
        (
            "[[0],null,null,9]\n[[1],null,null,9]\n[[1],null,null,9]\n".to_string(),
            String::new(),
            0
        )
    );
    Ok(())
}
