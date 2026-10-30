//! #5539: a name bound by `as` destructuring keeps the storage of the node it binds. jq's
//! `jv_equal` short-circuits on `jv_identical`, so `$x == $a[0]` is `true` even when the node
//! holds a NaN (`[nan] == [nan]` is `false`). The destructuring forms used to splice a rebuilt
//! literal instead, which is a new container. Every row's answer was captured from jq 1.7.1
//! (`/usr/bin/jq`).

use anyhow::Result;

use crate::{run_jq_null, run_jq_stdin};

fn answer(filter: &str) -> Result<String> {
    let (stdout, code) = run_jq_null(filter, &["-c"])?;
    assert_eq!(code, 0, "{filter}");
    Ok(stdout)
}

/// The issue's own table, plus the nested, `?//` and multi-name shapes of the same bind. Each
/// row was `false`.
#[test]
#[cfg_attr(
    feature = "unshared-containers",
    ignore = "holdout has no node identity or sharing (#5254)"
)]
fn test_destructured_container_keeps_its_storage_5539() -> Result<()> {
    for filter in [
        "[[nan]] as $a | $a as [$x] | $x == $a[0]",
        "[{\"k\":[nan]}] as $a | $a[0] as {k: $x} | $x == $a[0].k",
        "[[nan]] as $a | $a as [$x] ?// $x | $x == $a[0]",
        "[[[nan]]] as $a | $a as [[$x]] | $x == $a[0][0]",
        "[{\"a\":[nan]}] as $a | $a as [{a:$x}] | $x == $a[0].a",
        "[[nan]] as $a | [$a] as [[$x]] | $x == $a[0]",
        "[[nan]] as $a | $a as [$x] | [$x] == [$a[0]]",
    ] {
        assert_eq!(answer(filter)?, "true\n", "{filter}");
    }
    assert_eq!(
        answer("[[nan],[nan]] as $a | $a as [$x,$y] | [$x == $a[0], $y == $a[1], $x == $y]")?,
        "[true,true,false]\n"
    );
    Ok(())
}

/// A fresh container that merely looks equal is a different `jv`, so sharing must not make it
/// identical. (This row was `false` before too: it guards the change against over-sharing, it
/// is not evidence the change works.)
#[test]
fn test_destructured_container_is_not_a_fresh_one_5539() -> Result<()> {
    assert_eq!(
        answer("[[nan]] as $a | $a as [$x] | $x == [nan]")?,
        "false\n"
    );
    Ok(())
}

/// The generic evaluator's route, with a document to walk instead of `-n`: both evaluators end
/// in the same substitution, and neither may rebuild the container.
#[test]
#[cfg_attr(
    feature = "unshared-containers",
    ignore = "holdout has no node identity or sharing (#5254)"
)]
fn test_destructured_container_keeps_its_storage_on_a_document_5539() -> Result<()> {
    for filter in [
        "[[nan]] as $a | $a as [$x] | $x == $a[0]",
        "[{\"k\":[nan]}] as $a | $a[0] as {k: $x} | $x == $a[0].k",
        "[[nan]] as $a | $a as [$x] ?// $x | $x == $a[0]",
    ] {
        let (stdout, code) = run_jq_stdin(filter, "[1]", &["-c"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(stdout, "true\n", "{filter}");
    }
    Ok(())
}

/// A name the body only stores or adds keeps the literal, so the in-place assignment and
/// arithmetic steps stay in place (`destructured_variable_identity_gate_5539` pins the gate).
/// Answers match jq either way.
#[test]
fn test_destructured_container_stored_by_value_5539() -> Result<()> {
    for (filter, expected) in [
        (
            "[[1,2]] as $a | reduce $a[] as $r ({}; $r as [$x] | .a = $x | has(\"zz\")) | .",
            "false\n",
        ),
        (
            "[[[1],[2]]] as $a | reduce $a[] as [$p,$q] ([]; . + $p + $q)",
            "[1,2]\n",
        ),
    ] {
        assert_eq!(answer(filter)?, expected, "{filter}");
    }
    Ok(())
}

/// The control: a plain `as` bind of the same node already answered `true`.
#[test]
#[cfg_attr(
    feature = "unshared-containers",
    ignore = "holdout has no node identity or sharing (#5254)"
)]
fn test_plain_bind_control_5539() -> Result<()> {
    assert_eq!(
        answer("[[nan]] as $a | $a[0] as $x | $x == $a[0]")?,
        "true\n"
    );
    Ok(())
}
