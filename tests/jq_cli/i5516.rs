//! #5516: more shapes of #5487's class. A container keeps its storage through a computed-bound
//! full slice, an element of a shared slice, `limit`/`first` around a variable-headed pipe, and a
//! `reduce`/`foreach`/`last` step that hands its loop variable on, as jq's does: jq's `jv_equal`
//! short-circuits on `jv_identical`, so each row is `true` while `[nan] == [nan]` is `false`.
//! Every row's answer was captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_null;

fn answer(filter: &str) -> Result<String> {
    let (stdout, code) = run_jq_null(filter, &["-c"])?;
    assert_eq!(code, 0, "{filter}");
    Ok(stdout)
}

/// The issue's own table. #5515 as merged already answers every row, and rows 1, 2 and 4 were
/// right before it too; nothing pinned them.
#[test]
#[cfg_attr(
    feature = "unshared-containers",
    ignore = "holdout has no node identity or sharing (#5254)"
)]
fn test_computed_slices_and_limit_first_keep_the_variable_5516() -> Result<()> {
    for filter in [
        "[nan] as $a | ($a | .[0:($a|length)] | .) == $a",
        "[nan] as $a | 0 as $s | ($a | .[$s:] | .) == $a",
        "[nan] as $a | ($a | .[-9:] | .) == $a",
        "[nan,{\"a\":nan}] as $a | ($a | .[0:2] | .[1] | .) == $a[1]",
        "[nan] as $a | [limit(1; $a | . | .)] | .[0] == $a",
        "[nan] as $a | first($a | . | .) == $a",
    ] {
        assert_eq!(answer(filter)?, "true\n", "{filter}");
    }
    Ok(())
}

/// A fold step whose output is its container loop variable, or a node of it, hands on the
/// element itself, so a comparison after the fold sees one handle. `last(f)` is `reduce f as $x
/// (null; $x)`. Each row was `false`.
#[test]
#[cfg_attr(
    feature = "unshared-containers",
    ignore = "holdout has no node identity or sharing (#5254)"
)]
fn test_fold_step_hands_its_loop_variable_on_5516() -> Result<()> {
    for filter in [
        "[nan] as $a | last($a) == $a",
        "[nan] as $a | last($a | . | .) == $a",
        "[nan] as $a | last($a, $a) == $a",
        "[nan] as $a | last(range(2) | $a) == $a",
        "{\"a\":nan} as $a | last($a) == $a",
        "[nan] as $a | reduce $a as $v (null; $v) == $a",
        "[nan] as $a | reduce ($a | . | .) as $v (null; $v) == $a",
        "[nan] as $a | reduce $a as $v (null; [$v]) | .[0] == $a",
        "[nan] as $a | reduce $a as $v (null; {x: $v}) | .x == $a",
        "[[nan]] as $a | reduce $a as $v (null; $v[0]) == $a[0]",
        "[{\"a\":[nan],\"s\":1}] as $a | reduce $a[] as $r (null; $r.a) == $a[0].a",
        "[[nan]] as $a | reduce $a as [$x] (null; $x) == $a[0]",
        "[nan] as $a | reduce $a as $v (null; if true then $v else 0 end) == $a",
        "[nan] as $a | reduce $a as $v (null; $v // 0) == $a",
        "[nan] as $a | reduce $a as $v (null; try $v catch 0) == $a",
        "[nan] as $a | foreach $a as $v (0; $v) == $a",
        "[nan] as $a | foreach ($a | . | .) as $v (0; 0; $v) == $a",
        "[nan] as $a | foreach $a as $v (0; . + 1; [$v, .]) | .[0] == $a",
        "[nan] as $a | [foreach $a as $v (0; $v)] | .[0] == $a",
        // Through a binder, and as an error a by-value position raises.
        "[nan] as $a | reduce $a as $v (null; $v as $s | $s) == $a",
        "[[nan]] as $a | reduce $a as $v (null; reduce $v[] as $t (null; $t)) == $a[0]",
        "[nan] as $a | reduce $a as $v (null; try (error($v) and true) catch .) == $a",
        "[nan] as $a | reduce $a as $v (null; try (if error($v) then 1 else 0 end) catch .) == $a",
        "[nan] as $a | reduce $a as $v (null; try (error($v) as $x | 1) catch .) == $a",
    ] {
        assert_eq!(answer(filter)?, "true\n", "{filter}");
    }
    Ok(())
}

/// Controls: a fresh container is a new allocation in jq too, and `nan != nan` makes it unequal
/// to the variable.
#[test]
fn test_a_fresh_container_is_not_the_variable_5516() -> Result<()> {
    for filter in [
        "[nan] == [nan]",
        "[nan,1] as $a | ($a | .[0:($a|length - 1)] | .) == $a",
        "[nan] as $a | reduce $a as $v (null; [nan]) == $a",
        "[nan] as $a | last([nan]) == $a",
    ] {
        assert_eq!(answer(filter)?, "false\n", "{filter}");
    }
    Ok(())
}
