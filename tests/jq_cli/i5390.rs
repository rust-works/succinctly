//! #5390: `pick(f)` over a path program the interpreter runs (`recurse(f)` with an `f` that
//! is not a plain descent) folds its picks into one state in place, instead of running jq's
//! `reduce` body, which copied the state once per pick: 28 s at 80,001 nodes, where `..` took
//! 0.56 s. Rows were captured from jq 1.7.1 (`/usr/bin/jq`).

use std::time::{Duration, Instant};

use anyhow::Result;

use crate::{run_jq_full, run_jq_stdin};

const DOC: &str = r#"{"a":1,"b":{"c":[1,2,{"d":3}],"e":null},"f":[[1],[2,3]]}"#;

#[test]
fn test_pick_over_a_recurse_walk_matches_jq_5390() -> Result<()> {
    let picked = "{\"a\":1,\"b\":{\"c\":[1,2,{\"d\":3}]},\"f\":[[1],[2,3]]}\n";
    for (filter, want) in [
        (
            r#"pick(recurse(.[]? | .) | select(type == "number"))"#,
            picked,
        ),
        (
            r#"pick(recurse(.[]? | .; type != "null") | select(type == "number"))"#,
            picked,
        ),
        (
            r#"pick(.b.c[]? | select(type == "number"))"#,
            "{\"b\":{\"c\":[1,2]}}\n",
        ),
        ("pick(empty)", "null\n"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(DOC))?;
        assert_eq!(
            (stdout.as_str(), code),
            (want, 0),
            "`{filter}` -- {stderr:?}"
        );
    }
    // An error in the walk still raises, after the picks before it.
    let (stdout, stderr, code) = run_jq_full(&["-c", "pick(recurse(.[] | .))"], Some(DOC))?;
    assert_eq!((stdout.as_str(), code), ("", 5), "{stderr:?}");
    // The fold's own errors are jq's `reduce` body's: a write at a negative index of
    // the empty state, a read through a non-container. (`first(...)` is a dynamic
    // target, so these run on the interpreter.)
    for (doc, filter, message) in [
        (
            "[1,2]",
            "pick(first(.[-1], .[0]))",
            "Out of bounds negative array index",
        ),
        (
            "[1,2]",
            "pick(first(.[-5], .[0]))",
            "Out of bounds negative array index",
        ),
        (
            r#"{"a":5}"#,
            "pick(first(.a.b, .a))",
            "Cannot index number with string \"b\"",
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(doc))?;
        assert_eq!((stdout.as_str(), code), ("", 5), "`{filter}` -- {stderr:?}");
        assert!(stderr.contains(message), "`{filter}` -- {stderr:?}");
    }
    // Whole containers picked, and a pick that reads through `null`.
    for (doc, filter, want) in [
        (
            r#"{"a":{"b":1}}"#,
            "pick(first(.a, .a.b))",
            "{\"a\":{\"b\":1}}\n",
        ),
        (
            r#"{"a":null}"#,
            "pick(first(.a.b.c))",
            "{\"a\":{\"b\":{\"c\":null}}}\n",
        ),
        ("null", "pick(first(.[2]))", "[null,null,null]\n"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(doc))?;
        assert_eq!(
            (stdout.as_str(), code),
            (want, 0),
            "`{filter}` -- {stderr:?}"
        );
    }
    // A `?//` in the paths keeps jq's own body (the fold's error can be retried).
    let (stdout, code) = run_jq_stdin(r"pick(.a as [$p] ?// $p | .a)", DOC, &["-c"])?;
    assert_eq!((stdout.as_str(), code), ("{\"a\":1}\n", 0));
    Ok(())
}

/// 50,000 one-element arrays: the fold is linear, so this takes about a second in a debug
/// build, where a copy of the state per pick took about half a minute. The bound sits well
/// between the two, so a slow machine does not trip it but a quadratic fold does.
#[test]
fn test_pick_over_a_recurse_walk_is_not_quadratic_5390() -> Result<()> {
    let doc = format!("[{}]", vec!["[1]"; 50_000].join(","));
    let started = Instant::now();
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            r#"pick(recurse(.[]? | .) | select(type == "number")) | length"#,
        ],
        Some(doc.as_str()),
    )?;
    assert_eq!((stdout.as_str(), code), ("50000\n", 0), "{stderr:?}");
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "took {:?}",
        started.elapsed()
    );
    Ok(())
}
