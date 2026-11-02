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

/// A fold that stores the loop element through a slice or a delete: the answer follows jq's
/// root, `false` where it builds a new array and `true` where it keeps the input (a full-range
/// slice, a path through something absent). The fold gate used to keep the literal operand for
/// all of them.
#[test]
#[cfg_attr(
    feature = "unshared-containers",
    ignore = "holdout has no node identity or sharing (#5254)"
)]
fn test_fold_store_through_a_slice_or_delete_follows_jq_5761() -> Result<()> {
    for (filter, expected) in [
        ("{}; .x = ($v | del(.[5].x))", "true"),
        ("{}; .x = ($v | del(empty))", "true"),
        ("{}; .x = ($v | .[0:])", "true"),
        ("{}; .x = ($v | try del(.[5].x.y) catch 1)", "true"),
        ("{}; .x = ($v | delpaths([[5]]))", "false"),
        ("{}; .x = ($v | del(.[5]?))", "false"),
        ("{}; .x = ($v | del(.[5], .[6]))", "false"),
        ("{}; .x = ($v | .[1:2])", "false"),
        ("{}; .x = ($v | (.[5] |= empty))", "false"),
        ("{}; .x = ($v | del(.[0:0]))", "false"),
    ] {
        let program = format!("[[nan]] as $a | reduce $a[] as $v ({filter}) | .x == $a[0]");
        assert_eq!(answer(&program)?, format!("{expected}\n"), "{program}");
    }
    for (program, expected) in [
        (
            "[[nan]] as $a | reduce $a[] as $v (null; .[1:] |= $v) == $a[0]",
            "false",
        ),
        (
            "[[nan]] as $a | reduce $a[] as $v ([]; . + [$v | del(.[9])]) | .[0] == $a[0]",
            "false",
        ),
        (
            "[[nan]] as $a | reduce $a[] as $v ([]; . + ($v | del(.[9]))) == $a[0]",
            "false",
        ),
        (
            "[[nan]] as $a | reduce $a[] as $v (null; . + ($v | .[0:])) == $a[0]",
            "true",
        ),
    ] {
        assert_eq!(answer(program)?, format!("{expected}\n"), "{program}");
    }
    Ok(())
}

/// A fold that adds or defaults its loop element onto a `null` (or, for `//=`, `false`) start
/// keeps the element itself, as jq's `reduce` does: `add` is `reduce .[] as $x (null; . + $x)`.
/// These were `false` here for the compound spellings and `true` in jq 1.7.1.
#[test]
#[cfg_attr(
    feature = "unshared-containers",
    ignore = "holdout has no node identity or sharing (#5254)"
)]
fn test_fold_compound_store_over_a_null_state_keeps_the_element_5761() -> Result<()> {
    for (filter, expected) in [
        (
            "[nan] as $a | reduce $a as $v (null; . += $v) | . == $a",
            "true",
        ),
        (
            "[nan] as $a | reduce $a as $v (null; . //= $v) | . == $a",
            "true",
        ),
        (
            "[nan] as $a | reduce $a as $v (false; . //= $v) | . == $a",
            "true",
        ),
        (
            "[nan] as $a | [foreach $a as $v (null; . += $v; .)] | .[0] == $a",
            "true",
        ),
        (
            "[nan] as $a | [foreach $a as $v (null; . //= $v)] | .[0] == $a",
            "true",
        ),
        (
            "[[nan]] as $a | reduce $a[] as $v (null; . += $v) == $a[0]",
            "true",
        ),
        (
            "{\"a\":[nan]} as $a | reduce $a as $v (null; . //= $v) == $a",
            "true",
        ),
        // A second element adds to the first, a new container.
        (
            "[[nan],[nan]] as $a | reduce $a[] as $v (null; . += $v) == [nan,nan]",
            "false",
        ),
        // A truthy state is `//=`'s answer and is never replaced.
        (
            "[nan] as $a | reduce $a as $v ([]; . //= $v) | . == $a",
            "false",
        ),
        ("[nan] as $a | reduce $a as $v (1; . //= $v)", "1"),
        (
            "[nan] as $a | reduce $a as $v ({\"x\":1}; .x //= $v) | .x",
            "1",
        ),
        // Scalars, literals and operands that raise or yield several values are unchanged.
        ("reduce (-0) as $v (null; . += $v)", "-0"),
        (
            "reduce (100000000000000000000000000) as $v (null; . //= $v)",
            "100000000000000000000000000",
        ),
        ("reduce (1.000) as $v (null; . += $v)", "1.000"),
        ("reduce (\"s\") as $v (false; . //= $v)", "\"s\""),
        ("reduce (false) as $v (null; . //= $v)", "false"),
        ("reduce range(3) as $i (null; . += [$i])", "[0,1,2]"),
        ("reduce range(2) as $i (null; . += (1,2))", "4"),
        ("reduce range(2) as $i (null; . //= (1,2))", "2"),
        ("reduce range(2) as $i (1; . //= empty)", "null"),
        ("[foreach range(3) as $i (false; . //= $i; .)]", "[0,0,0]"),
        // Still `false` here and `true` in jq: a trailing identity stage after the store.
        (
            "[nan] as $a | reduce $a as $v (null; . += $v | .) | . == $a",
            "false",
        ),
        (
            "[nan] as $a | reduce $a as $v (null; . //= $v | .) | . == $a",
            "false",
        ),
        // A write after the store copies: the bound value is unchanged.
        (
            "[nan] as $a | reduce $a as $v (null; . += $v) | .[0] = 7 | [., $a]",
            "[[7],[null]]",
        ),
        (
            "[nan] as $a | reduce $a as $v (null; . //= $v) | .[0] = 7 | [., $a]",
            "[[7],[null]]",
        ),
        (
            "[[1]] as $a | reduce $a[] as $v (null; . += $v | .[0] = 9) | [., $a]",
            "[[9],[[1]]]",
        ),
    ] {
        assert_eq!(answer(filter)?, format!("{expected}\n"), "{filter}");
    }
    Ok(())
}

/// `//=`'s operand is read before the state is tested, so one that raises raises over a truthy
/// state too, as in jq.
#[test]
fn test_fold_alternative_store_evaluates_its_operand_first_5761() -> Result<()> {
    for filter in [
        "reduce range(2) as $i (1; . //= error(\"x\"))",
        "reduce range(2) as $i (null; . //= error(\"x\"))",
        "reduce range(2) as $i (null; . += error(\"x\"))",
    ] {
        let (stdout, stderr, code) = crate::run_jq_full(&["-nc", filter], None)?;
        assert_eq!(
            (stdout.as_str(), code, stderr.matches("jq: error").count()),
            ("", 5, 1),
            "{filter}: {stderr}"
        );
    }
    Ok(())
}
