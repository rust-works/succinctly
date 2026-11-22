//! #7720: a `?//` inside `map(f)`, `walk(f)` or `with_entries(f)` that is an object value (`{a:
//! map(. as [$x] ?// $x | $x)}`) is a `Term`, as any call is, and jq answers it; since #7602 spelled
//! such a call as jq's own `def` of it, the object-value check refused it at compile time ("object
//! value after ':' must be a term", exit 3) because it tested the parsed `Expr` variant. The check now
//! reads the keyword the value is written with, so a written `try`, `label` or `def` is still refused.
//!
//! Every expectation was captured from jq 1.7.1 (`/usr/bin/jq`); jq 1.8.2 agrees on every accepted
//! row. On the refused rows 1.8.2 differs where its grammar widened the object value (it accepts
//! `{a: try 1}`, `{a: -try 1}`, `{a: if ... end}`, `{a: reduce ...}`, `{a: map(1)?}`), a deliberate
//! 1.8 grammar change that keeps 1.7.1 as the reference; it refuses `label` and `def` as 1.7.1 does.

use anyhow::Result;

use crate::run_jq_full;

/// `(program, stdout)`: every row exits 0 with nothing on stderr.
static ANSWERED: &[(&str, &str)] = &[
    (
        "[[1],2] | {a: map(. as [$x] ?// $x | $x)}",
        "{\"a\":[1,2]}\n",
    ),
    (
        "[[1],2] | {\"k\": map(. as [$x] ?// $x | $x)}",
        "{\"k\":[1,2]}\n",
    ),
    (
        "[[1],2] | {(\"k\"): map(. as [$x] ?// $x | $x)}",
        "{\"k\":[1,2]}\n",
    ),
    (
        "[[1],2] | {a: 1, b: map(. as [$x] ?// $x | $x)}",
        "{\"a\":1,\"b\":[1,2]}\n",
    ),
    (
        "[[1],2] | . as $v | {$v, a: map(. as [$x] ?// $x | $x)}",
        "{\"v\":[[1],2],\"a\":[1,2]}\n",
    ),
    (
        "[[1],2] | {a: map(. as [$x] ?// $x | $x), b: map(. as [$x] ?// $x | $x)}",
        "{\"a\":[1,2],\"b\":[1,2]}\n",
    ),
    (
        "[[1],2] | {a: {b: map(. as [$x] ?// $x | $x)}}",
        "{\"a\":{\"b\":[1,2]}}\n",
    ),
    (
        "[[1],2] | {a: map(. as [$x] ?// $x | $x) | length}",
        "{\"a\":2}\n",
    ),
    (
        "[[1],[2]] | {a: map(select((. as [$a] ?// $a | $a) == 2))}",
        "{\"a\":[[2]]}\n",
    ),
    (
        "[[1],2] | {\"k\": walk(. as [$x] ?// $x | $x)}",
        "{\"k\":1}\n",
    ),
    (
        "[[1],2] | {k: walk(if type == \"array\" then . else . as [$x] ?// $x | $x + 1 end)}",
        "{\"k\":[[2],3]}\n",
    ),
    (
        "{\"a\":[1]} | {a: with_entries(.value as [$x] ?// $x | .value = $x)}",
        "{\"a\":{\"a\":1}}\n",
    ),
    (
        "[\"x\"] | {a: map(startswith([1,\"x\"] as [$a] ?// [$b,$a] | $a))}",
        "{\"a\":[true]}\n",
    ),
    (
        "[[1],2] | {a: [map(. as [$x] ?// $x | $x)]}",
        "{\"a\":[[1,2]]}\n",
    ),
    (
        "[[1],2] | {a: -(map(. as [$x] ?// $x | $x) | length)}",
        "{\"a\":-2}\n",
    ),
    ("{a: -(try 1)}", "{\"a\":-1}\n"),
    ("{a: (def f: 1; f)}", "{\"a\":1}\n"),
    ("{a: (label $f | 1)}", "{\"a\":1}\n"),
];

/// Object values jq 1.7.1 refuses at compile time (exit 3, nothing on stdout): a written `try`,
/// `label` or `def`, under unary `-` or after a `|` too, with or without a `?//` inside.
static REFUSED: &[&str] = &[
    "{a: try 1}",
    "{a: try 1 catch 2}",
    "{a: label $f | 1}",
    "{a: def f: 1; f}",
    "{a: -try 1}",
    "{a: 1 | try 1}",
    "{a: if true then 1 else 2 end}",
    "{a: reduce 1 as $x (0; .)}",
    "{a: map(1)?}",
    "{a: try map(. as [$x] ?// $x | $x)}",
    "{a: def f: map(. as [$x] ?// $x | $x); f}",
];

#[test]
fn a_retrying_bind_in_map_walk_or_with_entries_is_an_object_value_7720() -> Result<()> {
    for &(program, stdout) in ANSWERED {
        let got = run_jq_full(&["-nc", program], None)?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (stdout, "", 0),
            "{program:?}"
        );
    }
    Ok(())
}

#[test]
fn a_written_try_label_or_def_object_value_is_still_refused_7720() -> Result<()> {
    for &program in REFUSED {
        let (stdout, stderr, code) = run_jq_full(&["-nc", program], None)?;
        assert_eq!((stdout.as_str(), code), ("", 3), "{program:?}");
        assert!(
            stderr.ends_with("jq: 1 compile error\n"),
            "{program:?}: {stderr:?}"
        );
    }
    Ok(())
}
