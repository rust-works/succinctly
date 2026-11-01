//! #5761: a slice assigned onto `null` and a delete that names nothing return the handle of their
//! input where jq builds a new container, so `==` (which short-circuits on `jv_identical`, while
//! `[nan] == [nan]` is `false`) saw one array. Every answer below was captured from jq 1.7.1
//! (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_null;

fn answer(filter: &str) -> Result<String> {
    let (stdout, code) = run_jq_null(filter, &["-c"])?;
    assert_eq!(code, 0, "{filter}");
    Ok(stdout)
}

/// Each spelling was `true` here and is `false` in jq: the result is a new container, not the
/// array that was stored or deleted from.
#[test]
fn test_noop_delete_and_null_slice_store_build_a_new_array_5761() -> Result<()> {
    for filter in [
        // The issue's two roots.
        "[nan] as $a | null | .[1:] = $a | . == $a",
        "[nan] as $a | $a | del(.[5]) == $a",
        // Other spellings of a delete whose index names nothing.
        "[nan] as $a | $a | del(.[-5]) == $a",
        "[nan] as $a | $a | del(.[-0.5]) == $a",
        "[nan] as $a | $a | del(.[5]?) == $a",
        "[nan] as $a | $a | del(.[5+0]) == $a",
        "[nan] as $a | 5 as $i | $a | del(.[$i]) == $a",
        "[[nan]] as $a | $a | del(.[0][5]) == $a",
        "[nan] as $a | {\"a\":$a} | del(.a[5]) | .a == $a",
        // `|= empty` ends in the same `delpaths`.
        "[nan] as $a | $a | (.[5] |= empty) == $a",
        "[nan] as $a | $a | (.[-5] |= empty) == $a",
        "[nan] as $a | {\"a\":$a} | (.a[5] |= empty) | .a == $a",
        // Other slice stores over `null`.
        "[nan] as $a | null | (.[0:] = $a) == $a",
        "[nan] as $a | null | (.[1:2] = $a) == $a",
        "[nan] as $a | null | (.[1:] |= $a) == $a",
        "[nan] as $a | {} | (.x[1:] = $a) | .x == $a",
        // Already `false`: the sibling spellings that always copied.
        "[nan] as $a | $a | delpaths([[5]]) == $a",
        "[nan] as $a | $a | del(.[5], .[6]) == $a",
        "[nan] as $a | $a | del(.[5:7]) == $a",
        "[nan] as $a | null | setpath([{\"start\":1,\"end\":null}]; $a) == $a",
    ] {
        assert_eq!(answer(filter)?, "false\n", "{filter}");
    }
    Ok(())
}

/// What must not change: a path that runs through something absent keeps identity in jq, and the
/// new container is a shallow copy, so the elements inside it are still the same handles.
#[test]
#[cfg_attr(
    feature = "unshared-containers",
    ignore = "holdout has no node identity or sharing (#5254)"
)]
fn test_paths_that_name_nothing_keep_identity_5761() -> Result<()> {
    for filter in [
        "[nan] as $a | $a | del(.[5].x) == $a",
        "[nan] as $a | {\"a\":$a} | del(.a[5].x) | .a == $a",
        "[nan] as $a | $a | del(empty) == $a",
        "[[nan]] as $a | $a | del(.[5]) | .[0] == $a[0]",
        "[[nan]] as $a | $a | (.[5] |= empty) | .[0] == $a[0]",
        "[[nan]] as $a | null | .[0:] = $a | .[0] == $a[0]",
        "[nan] as $a | [$a, $a] | del(.[5]) | .[0] == .[1]",
    ] {
        assert_eq!(answer(filter)?, "true\n", "{filter}");
    }
    Ok(())
}

/// The value is the one the delete or store always gave.
#[test]
fn test_noop_delete_and_null_slice_store_keep_their_values_5761() -> Result<()> {
    for (filter, expected) in [
        ("[1,2,3] | del(.[5])", "[1,2,3]\n"),
        ("[1,2,3] | del(.[-5])", "[1,2,3]\n"),
        ("[1,2,3] | (.[5] |= empty)", "[1,2,3]\n"),
        ("null | del(.[5])", "null\n"),
        ("null | (.[5] |= empty)", "null\n"),
        ("null | (.[1:] |= empty)", "null\n"),
        ("null | (.[1:] = [1,2])", "[1,2]\n"),
        (
            "[1,2] as $a | $a | del(.[5]) | .[0] = 9 | [., $a]",
            "[[9,2],[1,2]]\n",
        ),
        (
            "[1,2] as $a | null | .[1:] = $a | .[0] = 9 | [., $a]",
            "[[9,2],[1,2]]\n",
        ),
    ] {
        assert_eq!(answer(filter)?, expected, "{filter}");
    }
    Ok(())
}
