//! #6659: a slice of an owned array is a view on the same storage, as jq's `jvp_array_slice` is,
//! not a copy of the range (ADR-0029). A loop that peels one element per step used to copy the
//! tail every step, so it was quadratic in time and memory where jq is linear: 233 s for the `def`
//! row at 100,000 elements, and `[while(...)]` aborted on an allocation failure.
//!
//! Every expected output here is jq 1.7.1's (`/usr/bin/jq -nc`). The loop rows run at 20,000
//! elements, which took seconds per row before the change and take milliseconds now, so a
//! regression to a copy shows as a slow test before it shows as a wrong answer.
//!
//! The identity rows read `==`, which short-circuits on `jv_identical` (#3069): two slices
//! holding a `nan` compare equal only when they are the same node.

use anyhow::Result;

use crate::run_jq_null;

fn jq_n(filter: &str) -> Result<String> {
    let (stdout, code) = run_jq_null(filter, &["-c"])?;
    assert_eq!(code, 0, "{filter}: {stdout}");
    Ok(stdout.trim_end().to_string())
}

#[test]
fn a_loop_that_peels_the_front_of_a_built_array_is_linear() -> Result<()> {
    let rows = [
        // The `def` form: the tail loop's condition is also settled without the bridge.
        (
            "[range(20000)] | def f: if length>10 then .[1:]|f else length end; f",
            "10",
        ),
        (
            "[range(20000)] | def g: if length>10 then .[:-1]|g else length end; g",
            "10",
        ),
        // Keeps every tail: one storage, 19,990 windows on it.
        (
            "[range(20000)] | [while(length>10; .[1:])] | length",
            "19990",
        ),
        ("[range(20000)] | until(length<10; .[1:]) | length", "9"),
        (
            "[range(20000)] | reduce range(length-10) as $i (.; .[1:]) | length",
            "10",
        ),
        (
            "[limit(3; [range(100000)] | recurse(.[1:])) | length]",
            "[100000,99999,99998]",
        ),
        ("[range(20000)] | [.[1:][]] | add", "199990000"),
        // A queue: the append writes into the view in place, as `jvp_array_write` does, so the
        // head is not compacted every round.
        (
            "[range(20000)] | reduce range(20000) as $i (.; .[1:] | . + [$i]) | [length, .[0], .[-1]]",
            "[20000,0,19999]",
        ),
    ];
    for (filter, want) in rows {
        assert_eq!(jq_n(filter)?, want, "{filter}");
    }
    Ok(())
}

#[test]
fn a_slice_of_a_built_array_has_jqs_identity() -> Result<()> {
    let rows = [
        // The same range of the same array is one node.
        ("[1,nan,2] as $a | $a[0:2] == $a[0:2]", "true"),
        ("[1,nan,2] as $a | $a[1:] == $a[1:]", "true"),
        ("[1,nan,2] as $a | $a[0:2][1:] == $a[1:2]", "true"),
        // The whole range is the array itself.
        ("[1,nan,2] as $a | $a[0:3] == $a", "true"),
        ("[1,nan,2] as $a | $a[1:][0:] == $a[1:]", "true"),
        // A different range is a different node, and so is a fresh empty one.
        ("[1,nan,2] as $a | $a[1:2] == $a[1:]", "false"),
        // jq's offset is 16 bits: from 65,536 on, a slice is a fresh copy each time.
        ("[range(65536)] + [nan] | .[65536:] == .[65536:]", "false"),
        (
            "[range(65536)] + [nan] | .[65535:][1:] == .[65535:][1:]",
            "false",
        ),
        (
            "[range(65536)] + [nan] | .[1:][65535:] == .[1:][65535:]",
            "false",
        ),
        // A fold's in-place cut moves the offset as jq's slice does: after 65,000 cuts a view
        // 535 further on is still a view, one 536 further on crosses 65,536 and is a copy.
        (
            "[range(70000)] + [nan] | reduce range(65000) as $i (.; .[1:]) | . as $s | [$s[536:] == $s[536:], $s[535:] == $s[535:]]",
            "[false,true]",
        ),
        // Past the copy the offset counts from 0 again, as the copy's does.
        (
            "[range(70000)] + [nan] | reduce range(66000) as $i (.; .[1:]) | . as $s | $s[3990:] == $s[3990:]",
            "true",
        ),
        // A path through a bound slice of a built document.
        (
            r#"{"a":[1,2,3]} | path(.a[0:2] as $y | .a[0:2] | $y)"#,
            r#"["a",{"start":0,"end":2}]"#,
        ),
        (
            r#"{"a":[1,2,3]} | path(.a[0:3] as $y | .a | $y)"#,
            r#"["a"]"#,
        ),
    ];
    for (filter, want) in rows {
        assert_eq!(jq_n(filter)?, want, "{filter}");
    }
    Ok(())
}

/// A write through a slice still leaves the array it was cut from alone, and an update of a slice
/// still splices its result back.
#[test]
fn a_write_through_a_slice_does_not_reach_the_array_it_was_cut_from() -> Result<()> {
    let rows = [
        (
            "[0,1,2,3] | .[1:3] as $s | ($s | .[0] = 9), .",
            "[9,2]\n[0,1,2,3]",
        ),
        ("[0,1,2,3] | (.[1:3] | .[0] = 9), .", "[9,2]\n[0,1,2,3]"),
        ("[0,1,2,3] | .[1:3] |= map(. * 10)", "[0,10,20,3]"),
        ("[0,1,2,3] | del(.[1:3][0])", "[0,2,3]"),
        ("[0,1,2,3] | .[1:] + [4]", "[1,2,3,4]"),
        ("[0,1,2,3] | .[1:] as $t | [$t[], .[0]]", "[1,2,3,0]"),
    ];
    for (filter, want) in rows {
        assert_eq!(jq_n(filter)?, want, "{filter}");
    }
    Ok(())
}

/// `succinctly jq ARGS` over `input` on standard input: stdout and stderr together, trimmed.
fn jq_doc(filter: &str, input: &str) -> Result<String> {
    let (stdout, stderr, _code) = crate::run_jq_full(&["-c", filter], Some(input))?;
    Ok(format!("{}{}", stdout.trim_end(), stderr.trim_end()))
}

/// A `def` tail loop settles its `if` condition from a built value's tree when it can (the
/// `length > 10` of the loops above), and every other condition keeps the evaluator's route: one
/// the pure grammar answers with an error, one over the document itself, one that yields nothing,
/// and one that raises through that route. Outputs are jq 1.7.1's.
#[test]
fn a_def_tail_loops_condition_answers_as_jq_on_every_route() -> Result<()> {
    // A built value whose condition raises in the pure grammar.
    let (stdout, stderr, code) = crate::run_jq_full(
        &[
            "-nc",
            "[range(3)] | def f: if .a > 1 then .[1:] | f else length end; f",
        ],
        None,
    )?;
    assert_eq!((stdout.as_str(), code), ("", 5));
    assert!(
        stderr.contains(r#"Cannot index array with string "a""#),
        "{stderr}"
    );
    // A condition that yields nothing ends the loop with no output.
    for filter in [
        "[range(3)] | def f: if (length | select(. > 5)) then .[1:] | f else length end; f",
        "[range(3)] | def f: if (.[0] | select(. > 5)) then .[1:] | f else length end; f",
    ] {
        assert_eq!(jq_n(filter)?, "", "{filter}");
    }
    // Over the document: the condition reads a document node.
    assert_eq!(
        jq_doc(
            "def f: if .a then .a | f else . end; f",
            r#"{"a":{"a":null}}"#
        )?,
        r#"{"a":null}"#
    );
    // Over the document, a condition that raises.
    let out = jq_doc("def f: if .a.b then .a | f else . end; f", r#"{"a":1}"#)?;
    assert!(
        out.contains(r#"Cannot index number with string "b""#),
        "{out}"
    );
    Ok(())
}

/// `del()`'s walk over a target that is not an array refuses as jq does, before anything pads.
#[test]
fn a_delete_through_an_index_of_a_non_array_raises_as_jq() -> Result<()> {
    for (filter, want) in [
        (
            r#"{"a":{"x":1}} | try del(.a[0].b) catch ."#,
            r#""Cannot index object with number""#,
        ),
        (
            r#"{"a":"s"} | try del(.a[0]) catch ."#,
            r#""Cannot index string with number""#,
        ),
    ] {
        assert_eq!(jq_n(filter)?, want, "{filter}");
    }
    Ok(())
}
