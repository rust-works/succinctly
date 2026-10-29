//! #5487: a pipe of two or more stages headed by a variable keeps the variable's storage, as jq's
//! does: jq's `jv_equal` short-circuits on `jv_identical`, so `[nan] as $a | ($a | . | .) == $a`
//! is `true` (while `[nan] == [nan]` is `false`). Every row's answer was captured from jq 1.7.1
//! (`/usr/bin/jq`) and fails on the parent commit.

use anyhow::Result;

use crate::run_jq_null;

fn answer(filter: &str) -> Result<String> {
    let (stdout, code) = run_jq_null(filter, &["-c"])?;
    assert_eq!(code, 0, "{filter}");
    Ok(stdout)
}

#[test]
#[cfg_attr(
    feature = "unshared-containers",
    ignore = "holdout has no node identity or sharing (#5254)"
)]
fn test_variable_headed_pipe_is_identical_to_the_variable_5487() -> Result<()> {
    for filter in [
        // `==` on either side, an array and an object.
        "[nan] as $a | ($a | . | .) == $a",
        "[nan] as $a | $a == ($a | . | .)",
        "{\"a\":nan} as $a | ($a | . | .) == $a",
        // Rebinding the pipe's value.
        "[nan] as $a | ($a | . | .) as $b | $b == $a",
        // A full-range slice is the array itself in jq, whatever follows it.
        "[nan] as $a | ($a | .[0:] | .) == $a",
        "[nan,1] as $a | ($a | .[0:2] | .) == $a",
        "[nan] as $a | ($a | .[0:5] | .) == $a",
        // An empty array has no whole to be identical to, and is `true` for the same reason a
        // bare `[] == []` is.
        "[] as $a | ($a | .[0:] | .) == $a",
    ] {
        assert_eq!(answer(filter)?, "true\n", "{filter}");
    }
    Ok(())
}

#[test]
#[cfg_attr(
    feature = "unshared-containers",
    ignore = "holdout has no node identity or sharing (#5254)"
)]
fn test_variable_headed_pipe_inside_a_generator_5487() -> Result<()> {
    assert_eq!(
        answer("[nan] as $a | [range(0;3) | ($a | . | .)] | map(. == $a)")?,
        "[true,true,true]\n"
    );
    Ok(())
}

#[test]
#[cfg_attr(
    feature = "unshared-containers",
    ignore = "holdout has no node identity or sharing (#5254)"
)]
fn test_shapes_that_already_kept_the_handle_still_do_5487() -> Result<()> {
    for filter in [
        "[nan] as $a | ($a | .) == $a",
        "[nan] as $a | (($a | .) | .) == $a",
        "[nan] as $a | [$a | . | .] | .[0] == $a",
        "[nan] | . as $a | (. | . | .) == $a",
    ] {
        assert_eq!(answer(filter)?, "true\n", "{filter}");
    }
    Ok(())
}

#[test]
fn test_a_partial_slice_is_a_new_array_5487() -> Result<()> {
    // Only a slice naming every element is the array: these are new arrays in jq too, and
    // `nan != nan` makes them unequal to the variable.
    for filter in [
        "[nan,1] as $a | ($a | .[1:] | .) == $a",
        "[nan,1] as $a | ($a | .[:-1] | .) == $a",
    ] {
        assert_eq!(answer(filter)?, "false\n", "{filter}");
    }
    Ok(())
}
