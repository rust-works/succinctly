//! #5540: a fold step that stores its container loop variable by assignment or arithmetic hands
//! on the element itself, as jq's `reduce` does, so a later `==` sees one handle
//! (`jv_identical`) while `[nan] == [nan]` is `false`. #5516 left these steps out because they
//! run in place on the owned accumulator. Every row's answer was captured from jq 1.7.1
//! (`/usr/bin/jq`).

use anyhow::Result;

use crate::{run_jq_full, run_jq_null};

const DEF: &str = "def f(n): if n == 0 then 0 else [f(n-1)] end;";

fn answer(filter: &str) -> Result<String> {
    let (stdout, code) = run_jq_null(filter, &["-c"])?;
    assert_eq!(code, 0, "{filter}");
    Ok(stdout)
}

/// The issue's four rows, and the other spellings of a step that stores the variable. Each was
/// `false` here and `true` in jq.
#[test]
#[cfg_attr(
    feature = "unshared-containers",
    ignore = "holdout has no node identity or sharing (#5254)"
)]
fn test_fold_step_that_stores_its_loop_variable_keeps_the_element_5540() -> Result<()> {
    for filter in [
        // jq's own `def add: reduce .[] as $x (null; . + $x);`
        "[[nan]] as $a | reduce $a[] as $v (null; . + $v) == $a[0]",
        "[[nan]] as $a | reduce $a[] as $v ([]; . + [$v]) | .[0] == $a[0]",
        "[nan] as $a | reduce $a as $v ({}; .x = $v) | .x == $a",
        "[nan] as $a | reduce $a as $v ({}; .x |= $v) | .x == $a",
        "[nan] as $a | reduce $a as $v ({}; .x //= $v) | .x == $a",
        "[nan] as $a | reduce $a as $v ({}; .x += $v) | .x == $a",
        "[nan] as $a | reduce $a as $v ({}; .[\"k\"] = $v) | .k == $a",
        "[nan] as $a | reduce $a as $v ([]; . + [$v, 1]) | .[0] == $a",
        "{\"a\":nan} as $a | reduce $a as $v ([]; . + [$v]) | .[0] == $a",
        "[nan] as $a | [foreach $a as $v (null; . + $v; .)] | .[0] == $a",
        "[nan] as $a | reduce $a as $v ({}; .x = $v | .y = $v) | .x == .y",
        // A state-reading filter and a non-first pipe stage keep the element's handle too.
        "[nan] as $a | reduce $a as $v ({}; .x |= ((. // []) + [$v])) | .x[0] == $a",
        "[nan] as $a | reduce $a as $v ({}; .n += 1 | .rows += [$v]) | .rows[0] == $a",
        "[nan] as $a | reduce $a as $v ({}; .n += 1 | .l = .l + [$v]) | .l[0] == $a",
        // The builtin, and an outer variable, were already `true`.
        "[[nan]] as $a | $a | add == $a[0]",
        "[nan] as $a | reduce 1 as $_ ({}; .x = $a) | .x == $a",
    ] {
        assert_eq!(answer(filter)?, "true\n", "{filter}");
    }
    Ok(())
}

/// A slice assigned onto `null` and an out-of-range `del` build a new container, as jq does (#5761),
/// so a store through either is `false` under `==` in a fold too.
#[test]
fn test_store_through_a_slice_or_del_stays_unshared_5761() -> Result<()> {
    for filter in [
        "[nan] as $a | reduce $a as $v (null; .[1:] = $v) | . == $a",
        "[nan] as $a | reduce $a as $v (null; .[1:] |= $v) | . == $a",
        "[[nan]] as $a | reduce $a[] as $v ({}; .x = ($v | del(.[5]))) | .x == $a[0]",
        "[[nan]] as $a | reduce $a[] as $v ({}; .x |= ($v | del(.[-5]))) | .x == $a[0]",
    ] {
        assert_eq!(answer(filter)?, "false\n", "{filter}");
    }
    Ok(())
}

/// A container built around the variable, or equal to it, is a new one: only the element itself
/// is shared.
#[test]
fn test_fold_step_that_builds_a_fresh_container_stays_unequal_5540() -> Result<()> {
    for filter in [
        "[nan] as $a | reduce $a as $v ({}; .x = $v) | .x == [nan]",
        "[[nan]] as $a | reduce $a[] as $v (null; . + $v) == [nan]",
        "[[nan]] as $a | reduce $a[] as $v ([]; . + [$v]) | .[0] == [nan]",
        "[[nan],[nan]] as $a | reduce $a[] as $v (null; . + $v) == [nan,nan]",
    ] {
        assert_eq!(answer(filter)?, "false\n", "{filter}");
    }
    Ok(())
}

/// Storing the element is one handle, not an alias: a later write to the accumulator copies, and
/// the bound value is unchanged. The values are those of an unshared fold.
#[test]
fn test_writes_after_the_stored_element_do_not_reach_the_variable_5540() -> Result<()> {
    for (filter, expected) in [
        (
            "[nan] as $a | reduce $a as $v ({}; .x = $v | .x[0] = 1) | $a",
            "[null]\n",
        ),
        (
            "[nan] as $a | reduce $a as $v ([]; . + [$v] | .[0][0] = 1) | $a",
            "[null]\n",
        ),
        (
            "[[1],[2]] as $a | reduce $a[] as $v (null; . + $v)",
            "[1,2]\n",
        ),
        (
            "[[1],[2]] as $a | reduce $a[] as $v ({}; .[$v|tostring] = $v)",
            "{\"[1]\":[1],\"[2]\":[2]}\n",
        ),
        (
            "[{\"a\":1},{\"b\":2}] as $a | reduce $a[] as $v ({}; . + $v)",
            "{\"a\":1,\"b\":2}\n",
        ),
        (
            "{\"a\":[1]} as $o | [reduce range(2) as $i ($o; .a += [$i]), $o]",
            "[{\"a\":[1,0,1]},{\"a\":[1]}]\n",
        ),
        (
            "[{\"a\":{\"b\":1}}] as $a | [reduce $a[] as $v ({}; .x = $v | .x *= {\"a\":{\"c\":2}}), $a]",
            "[{\"x\":{\"a\":{\"b\":1,\"c\":2}}},[{\"a\":{\"b\":1}}]]\n",
        ),
        (
            "[{\"a\":1,\"b\":2}] as $a | [reduce $a[] as $v ({}; .x = $v | del(.x.a)), $a]",
            "[{\"x\":{\"b\":2}},[{\"a\":1,\"b\":2}]]\n",
        ),
        (
            "{\"u\":[{\"n\":\"x\",\"t\":[1]}]} | . as $d | [reduce $d.u[] as $r ({}; .a = $r | .b = $r | .a.t += [7]), $d]",
            "[{\"a\":{\"n\":\"x\",\"t\":[1,7]},\"b\":{\"n\":\"x\",\"t\":[1]}},{\"u\":[{\"n\":\"x\",\"t\":[1]}]}]\n",
        ),
        (
            "[[1],[2]] as $a | [reduce $a[] as $v ([]; . + [$v] | .[0] += [8]), $a]",
            "[[[1,8,8],[2]],[[1],[2]]]\n",
        ),
        // The embed table is live (`. as $d`), and the element is a node of the document.
        (
            "{\"u\":[{\"n\":\"x\",\"t\":[1]},{\"n\":\"y\",\"t\":[2]}]} | . as $d | [reduce $d.u[] as $r ({}; .[$r.n] = $r | .[$r.n].t += [0]), $d]",
            "[{\"x\":{\"n\":\"x\",\"t\":[1,0]},\"y\":{\"n\":\"y\",\"t\":[2,0]}},{\"u\":[{\"n\":\"x\",\"t\":[1]},{\"n\":\"y\",\"t\":[2]}]}]\n",
        ),
    ] {
        assert_eq!(answer(filter)?, expected, "{filter}");
    }
    Ok(())
}

/// A step that stores the variable hands the element on, so nothing walks a value built past the
/// 384-level ceiling when it is spliced back; jq 1.7.1 answers `1`, on both routes. #5526 had
/// refused it. The rebinding forms (`$x as $n | .a = $n`) keep the evaluator, which shares too.
#[test]
#[cfg_attr(
    feature = "unshared-containers",
    ignore = "holdout has no node identity or sharing (#5254)"
)]
fn test_fold_that_stores_a_deep_loop_variable_answers_as_jq_does_5540() -> Result<()> {
    for body in [
        "reduce f(400) as $x ([]; . + [$x]) | length",
        "reduce f(400) as $x ({}; .a = $x) | .a | length",
        "reduce f(400) as $x ({}; if $x == $x then ($x as $n | .a = $n) else . end) | .a | length",
    ] {
        let program = format!("{DEF} {body}");
        for (args, input) in [
            (&["-nc", program.as_str()][..], None),
            (&["-c", program.as_str()][..], Some("{}")),
        ] {
            let (stdout, stderr, code) = run_jq_full(args, input)?;
            assert_eq!(
                (stdout.as_str(), code),
                ("1\n", 0),
                "{body} {args:?}: stderr={stderr:?}"
            );
        }
    }
    Ok(())
}

/// The review of PR #5739 found a step that rebinds the stored container with `as`
/// rebuilding it as a literal, which lost the handle (`false` for `true`) and, for a value past
/// the ceiling, refused. Each row was `true`/`[null]`/`1` in jq 1.7.1 and on the merge base.
#[test]
#[cfg_attr(
    feature = "unshared-containers",
    ignore = "holdout has no node identity or sharing (#5254)"
)]
fn test_as_rebind_of_a_stored_loop_variable_keeps_the_element_5540() -> Result<()> {
    for (filter, expected) in [
        (
            "[nan] as $a | reduce $a as $v ({}; if $v == $v then ($v as $n | .x = $n) else . end) | .x == $a",
            "true\n",
        ),
        (
            "[nan] as $a | reduce $a as $v ({}; if $v == $v then ($v as $n | .y = (if $n == $n then $n else 1 end)) else . end) | .y",
            "[null]\n",
        ),
    ] {
        assert_eq!(answer(filter)?, expected, "{filter}");
    }
    Ok(())
}
