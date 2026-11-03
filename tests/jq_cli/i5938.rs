//! #5938: a field read through an `as` rebind of a fold's loop variable (`$r as $s | .a = $s.items`)
//! stored a rebuilt container where jq stores the member itself, so `==` (which short-circuits on
//! `jv_identical`, while `[nan] == [nan]` is `false`) saw a new array. Every answer below was
//! captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_null;

fn answer(filter: &str) -> Result<String> {
    let (stdout, code) = run_jq_null(filter, &["-c"])?;
    assert_eq!(code, 0, "{filter}");
    Ok(stdout)
}

/// Each spelling was `false` here and is `true` in jq: the stored member is the element's own array.
#[cfg_attr(
    feature = "unshared-containers",
    ignore = "holdout has no node identity or sharing (#5254)"
)]
#[test]
fn test_rebound_loop_variable_field_read_keeps_the_members_identity_5938() -> Result<()> {
    for filter in [
        // The issue's row.
        "[{\"items\":[nan]}] as $in | reduce $in[] as $r ({}; $r as $s | .a = $s.items) | .a == $in[0].items",
        // The other stores a field read reaches.
        "[{\"items\":[nan]}] as $in | reduce $in[] as $r ({}; $r as $s | .a += $s.items) | .a == $in[0].items",
        "[{\"n\":\"k\",\"items\":[nan]}] as $in | reduce $in[] as $r ({}; $r as $s | .[$s.n] = $s.items) | .k == $in[0].items",
        "[{\"items\":[nan]}] as $in | foreach $in[] as $r ({}; $r as $s | .a = $s.items; .a == $in[0].items)",
        // A rebind of a rebind, and a parenthesised source.
        "[{\"items\":[nan]}] as $in | reduce $in[] as $r ({}; $r as $s | $s as $t | .a = $t.items) | .a == $in[0].items",
        "[{\"items\":[nan]}] as $in | reduce $in[] as $r ({}; ($r) as $s | .a = $s.items) | .a == $in[0].items",
        // Beside a read of the key.
        "[{\"n\":\"k\",\"items\":[nan]}] as $in | reduce $in[] as $r ({}; $r as $s | $s.n as $k | .[$k] = $s.items) | .k == $in[0].items",
        // The whole variable, which already matched.
        "[{\"items\":[nan]}] as $in | reduce $in[] as $r ({}; $r as $s | .a = $s) | .a == $in[0]",
    ] {
        assert_eq!(answer(filter)?, "true\n", "{filter}");
    }
    Ok(())
}

/// Growing the stored member afterwards builds a new container, as in jq.
#[test]
fn test_rebound_loop_variable_field_read_grown_afterwards_is_a_new_container_5938() -> Result<()> {
    assert_eq!(
        answer(
            "[{\"items\":[nan]}] as $in | reduce $in[] as $r ({}; $r as $s | .a = $s.items | .a += [1]) | .a == $in[0].items"
        )?,
        "false\n"
    );
    Ok(())
}

/// The values stored are the same with and without the rebind.
#[test]
fn test_rebound_loop_variable_field_read_stores_the_same_values_5938() -> Result<()> {
    for (filter, expected) in [
        (
            "[{\"n\":\"a\",\"v\":[1]},{\"n\":\"b\",\"v\":{\"x\":2}}] | reduce .[] as $r ({}; $r as $s | .[$s.n] = $s.v)",
            "{\"a\":[1],\"b\":{\"x\":2}}\n",
        ),
        (
            "[{\"v\":[1]},{\"v\":[2]}] | reduce .[] as $r ([]; $r as $s | . + $s.v)",
            "[1,2]\n",
        ),
        (
            "[{\"v\":[1]},{\"v\":[2]}] | [foreach .[] as $r ([]; $r as $s | . + $s.v; length)]",
            "[1,2]\n",
        ),
    ] {
        assert_eq!(answer(filter)?, expected, "{filter}");
    }
    Ok(())
}
