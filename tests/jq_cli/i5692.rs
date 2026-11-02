//! #5692: a path ending on, or passing through, a container named argument (`--argjson`,
//! `--slurpfile`) is certified as jq certifies the variable's `jv`: `$d | path($d)` is `[]`,
//! `$d | del($d)` is `null` and `$d | path($d.a)` is `["a"]`. The argument used to be spliced as a
//! literal with an identity per occurrence, which refused all three. A path over an equal value
//! that is not the argument still refuses. Outputs are captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

/// The `--slurpfile s` file: two documents, so `$s` is `[{"a":1},[2]]`.
const SLURPED: &str = "{\"a\":1}\n[2]\n";

/// Run `filter` under `-nc --argjson d <d> --slurpfile s <SLURPED>`: `(stdout, stderr, code)`.
fn run(d: &str, filter: &str) -> Result<(String, String, i32)> {
    let dir = tempfile::tempdir()?;
    let slurped = dir.path().join("s.json");
    std::fs::write(&slurped, SLURPED)?;
    run_jq_full(
        &[
            "-nc",
            "--argjson",
            "d",
            d,
            "--slurpfile",
            "s",
            slurped.to_str().unwrap(),
            filter,
        ],
        None,
    )
}

/// `(--argjson d, filter, stdout)`, every one exiting 0.
#[test]
#[cfg_attr(
    feature = "unshared-containers",
    ignore = "holdout has no node identity or sharing (#5254)"
)]
fn a_path_through_a_named_argument_is_the_arguments_own_5692() -> Result<()> {
    let nested = r#"{"a":{"b":[1,{"c":2}]}}"#;
    for (d, filter, expected) in [
        (r#"{"a":1}"#, "$d | path($d)", "[]\n"),
        (r#"{"a":1}"#, "$d | del($d)", "null\n"),
        (r#"{"a":1}"#, "$d | path($d.a)", "[\"a\"]\n"),
        (r#"{"a":"s"}"#, "$d | path($d.a)", "[\"a\"]\n"),
        (
            r#"{"a":1}"#,
            "$d | try path($d.b.c) catch .",
            "[\"b\",\"c\"]\n",
        ),
        (r#"{"a":1}"#, "$d | path(getpath([\"a\"]))", "[\"a\"]\n"),
        ("[1,2]", "$d as $x | $d | path($x)", "[]\n"),
        ("[1,2]", "[$d] | path(.[0] | $d)", "[0]\n"),
        (nested, "$d | path($d.a.b[1].c)", "[\"a\",\"b\",1,\"c\"]\n"),
        (nested, "$d | del(.a.b[0])", "{\"a\":{\"b\":[{\"c\":2}]}}\n"),
        (nested, "$d | del(.. | numbers?)", "{\"a\":{\"b\":[{}]}}\n"),
        (
            nested,
            "$d | pick(.a.b[1].c)",
            "{\"a\":{\"b\":[null,{\"c\":2}]}}\n",
        ),
        (nested, "$d | [paths(..)] | length", "13\n"),
        (
            "[1,2]",
            "$d | walk(if type == \"number\" then . + 1 else . end)",
            "[2,3]\n",
        ),
        (r#"{"a":1}"#, "$s | path($s[0])", "[0]\n"),
        (r#"{"a":1}"#, "$s | del($s[1][0])", "[{\"a\":1},[]]\n"),
    ] {
        let (stdout, stderr, code) = run(d, filter)?;
        assert_eq!((stdout.as_str(), code), (expected, 0), "{filter}: {stderr}");
    }
    Ok(())
}

/// A path over a value equal to the argument but not it, or reached another way, is refused with
/// jq's own text: `(--argjson d, filter, stderr)`, every one exiting 5.
#[test]
fn a_path_over_a_value_that_is_not_the_argument_still_refuses_5692() -> Result<()> {
    for (d, filter, expected) in [
        (
            r#"{"a":1}"#,
            r#"{"a":1} | path($d)"#,
            r#"jq: error (at <unknown>): Invalid path expression with result {"a":1}"#,
        ),
        (
            r#"{"a":1}"#,
            r#"$d | {"a":1} | path($d)"#,
            r#"jq: error (at <unknown>): Invalid path expression with result {"a":1}"#,
        ),
        (
            r#"{"a":"s"}"#,
            "$d | path(.a | $d.a)",
            r#"jq: error (at <unknown>): Invalid path expression near attempt to access element "a" of {"a":"s"}"#,
        ),
        (
            r#"{"a":1}"#,
            "$s[0] | path($s[0].a)",
            r#"jq: error (at <unknown>): Invalid path expression near attempt to access element 0 of [{"a":1},[2]]"#,
        ),
        (
            r#"{"a":1}"#,
            "$d | reduce range(2) as $i (.; del($d.a))",
            r#"jq: error (at <unknown>): Invalid path expression near attempt to access element "a" of {"a":1}"#,
        ),
    ] {
        let (stdout, stderr, code) = run(d, filter)?;
        assert_eq!((stdout.as_str(), code), ("", 5), "{filter}: {stderr}");
        assert_eq!(stderr.trim_end(), expected, "{filter}");
    }
    Ok(())
}
