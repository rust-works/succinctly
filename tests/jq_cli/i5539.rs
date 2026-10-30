//! #5539: a name bound by `as` destructuring keeps the storage of the node it binds. jq's
//! `jv_equal` short-circuits on `jv_identical`, so `$x == $a[0]` is `true` even when the node
//! holds a NaN (`[nan] == [nan]` is `false`). The destructuring forms used to splice a rebuilt
//! literal instead, which is a new container. Every row's answer was captured from jq 1.7.1
//! (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_null;

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

/// What must stay `false`: a fresh container that merely looks equal is a different `jv`, and a
/// fold that destructures a container's elements binds the NaN scalar inside, not the container.
#[test]
fn test_destructured_container_is_not_a_fresh_one_5539() -> Result<()> {
    for filter in [
        "[[nan]] as $a | $a as [$x] | $x == [nan]",
        "[[nan]] as $a | reduce $a[] as [$x] (null; $x) == $a[0]",
    ] {
        assert_eq!(answer(filter)?, "false\n", "{filter}");
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
