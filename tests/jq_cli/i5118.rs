//! #5118: a slice past offset 65,535 of a value that is itself a view of a longer array.

use anyhow::Result;

use crate::run_jq_full;

// jq's slice is a view of the array's buffer, with a 16-bit offset: from 65,536 it copies. The resolver records every node
// below the root at offset 0, so `[range(70003)] | .[60000:] | del(.[6000:] as $x | .[6000:] | $x)` deleted through a
// slice jq refuses. Once a slice has been resolved against a container of more than 65,535 elements, every slice that
// starts past 0 is read as possibly copied: loud where jq refuses, and over-refusing the views jq still shares.

/// #5118: the rows jq refuses and `main` answered (a view whose offset reaches 65,536 through a second slice, below a field,
/// after a write and after `+`). Each must refuse, as jq does (exit 5, no output).
///
/// #5223: the path-mode interpreter carries jq's register, so the rows the shipped resolver
/// refused here now hold jq 1.7.1's answers.
#[test]
fn test_5118_a_slice_past_65535_of_a_view_is_refused_as_jq_refuses() -> Result<()> {
    for filter in [
        "[range(70003)] | .[60000:] | del(.[6000:] as $x | .[6000:] | $x) | length",
        "[range(70003)] | .[60000:] | del(.[5536:] as $x | .[5536:] | $x) | length",
        "{a: ([range(70003)] | .[60000:])} | del(.a[6000:] as $x | .a[6000:] | $x) | .a | length",
        "[range(70003)] | .[60000:] | . + [] | del(.[6000:] as $x | .[6000:] | $x) | length",
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert!(
            code == 5 && stdout.is_empty() && stderr.contains("Invalid path expression"),
            "`{filter}`: jq refuses, got {stdout:?} {stderr:?} exit {code}"
        );
    }
    Ok(())
}

/// #5223: a write to a view keeps jq's buffer when nothing else holds it (`jvp_array_write` writes in place), so the view
/// keeps its offset of 60,000 and the second slice reaches 66,000 and is copied: jq refuses. The path-mode interpreter's
/// write builds a new array at offset 0, so the second slice is a view and the path answers. Recorded in
/// `docs/compliance/jq/limitations.md`.
#[cfg(not(feature = "unshared-containers"))]
#[test]
fn test_5118_a_written_view_answers_where_jq_refuses_5223() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-nc",
            "[range(70003)] | .[60000:] | .[0] = 0 | del(.[6000:] as $x | .[6000:] | $x) | length",
        ],
        None,
    )?;
    assert_eq!((stdout.as_str(), code), ("6000\n", 0), "{stderr}");
    Ok(())
}

/// #5118: small arrays are untouched by the gate, so the rows that match jq keep matching (`[1,2,3]`, and a slice of a
/// 70,003-element array that stays below the 65,536 offset answers as jq does or refuses).
#[test]
fn test_5118_small_arrays_still_answer_as_jq_does() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-nc", "[1,2,3] | del(.[1:] as $x | .[1:] | $x)"], None)?;
    assert_eq!((stdout.as_str(), code), ("[1]\n", 0), "{stderr}");
    // The gate stays down for an array of up to 65,536 elements (a copy needs a buffer longer than that), and a long
    // string does not raise it.
    for filter in [
        "[range(65536)] | del(.[6000:] as $x | .[6000:] | $x) | length",
        r#"("x" * 70000 | .[1:] | length) as $n | [range(10)] | del(.[6:] as $x | .[6:] | $x) | length"#,
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        let jq = if filter.starts_with("[range") {
            "6000\n"
        } else {
            "6\n"
        };
        assert_eq!((stdout.as_str(), code), (jq, 0), "`{filter}`: {stderr}");
    }
    for filter in [
        "[range(70003)] | del(.[6000:] as $x | .[6000:] | $x) | length",
        "[range(70003)] | .[60000:] | del(.[5535:] as $x | .[5535:] | $x) | length",
    ] {
        // jq answers 6000 and 5535; the gate over-refuses them (loud) rather than guess a view's offset.
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert!(
            (code == 0 && (stdout == "6000\n" || stdout == "5535\n"))
                || (code == 5 && stdout.is_empty()),
            "`{filter}`: {stdout:?} {stderr:?} exit {code}"
        );
    }
    Ok(())
}
