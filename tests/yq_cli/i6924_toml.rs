//! #6924: `succinctly yq -o toml`, a port of yq's TOML encoder: a root mapping's attributes first, then its
//! tables and arrays of tables, a blank line before a table that follows an attribute, keys bare or
//! Go-quoted, strings Go-quoted (`%q`), `null` left out of a table and `""` inside an array. Every answer
//! was captured from yq v4.53.3; a row is kept only when both agree on stdout and the exit status. Left out
//! because they still differ (see `docs/compliance/yq/limitations.md`): a comment, a boolean written
//! `True` or `yes`, the YAML 1.1 integers `1_000` and `0b11`, and a timestamp or `!!binary` scalar.

use anyhow::Result;

use crate::run_yq_stdin;

/// (input, yq's stdout, yq's exit status).
const ROWS: &[(&str, &str, i32)] = &[
    (
        "a: 1\nb: x\nc: true\nd: null\ne: 1.5\n",
        "a = 1\nb = \"x\"\nc = true\ne = 1.5\n",
        0,
    ),
    (
        "a: [1, 2]\nb: []\nc: [[1], [2, 3]]\nd: [{x: 1}, {y: 2}]\n",
        "a = [1, 2]\nb = []\nc = [[1], [2, 3]]\n\n[[d]]\nx = 1\n[[d]]\ny = 2\n",
        0,
    ),
    ("t: {a: 1, u: {b: 2}}\n", "[t]\na = 1\n\n[t.u]\nb = 2\n", 0),
    ("t: {u: {v: {a: 1}}}\n", "[t.u.v]\na = 1\n", 0),
    ("a: 1\nt: {b: 2}\nc: 3\n", "a = 1\nc = 3\n\n[t]\nb = 2\n", 0),
    ("t: [{a: 1}, {a: 2}]\n", "[[t]]\na = 1\n[[t]]\na = 2\n", 0),
    (
        "t: {u: [{a: 1, v: {b: 2}}]}\n",
        "[[t.u]]\na = 1\n\n[t.u.v]\nb = 2\n",
        0,
    ),
    (
        "\"a b\": 1\nc.d: 2\né: 3\nx-y: 4\n\"1\": 5\n",
        "\"a b\" = 1\n\"c.d\" = 2\n\"é\" = 3\nx-y = 4\n1 = 5\n",
        0,
    ),
    ("t: {}\n", "[t]\n", 0),
    ("t: {u: {}}\n", "[t.u]\n", 0),
    (
        "a: \"q\\\"r\\\\s\\ne\\t\\u0001é日本\\u00a0\\u2028\\u200b\\ufeff\\U0001F600\\ue000\"\n",
        "a = \"q\\\"r\\\\s\\ne\\t\\x01é日本\\u00a0\\u2028\\u200b\\ufeff😀\\ue000\"\n",
        0,
    ),
    ("a: [{x: 1}, 2]\n", "a = [{ x = 1 }, 2]\n", 0),
    ("a: [~, 1]\n", "a = [\"\", 1]\n", 0),
    ("a: {b: ~}\n", "", 0),
    ("t: {a: ~}\n", "", 0),
    ("a: [{k: [{z: 1}]}]\n", "[[a]]\n[[a.k]]\nz = 1\n", 0),
    (
        "a: x\nt: [{a: 1}]\nu: [1, 2]\n",
        "a = \"x\"\nu = [1, 2]\n\n[[t]]\na = 1\n",
        0,
    ),
    ("t: {a: [{b: 1}]}\n", "[[t.a]]\nb = 1\n", 0),
    ("[1]\n", "", 1),
    ("hello\n", "hello\n", 0),
    ("null\n", "null\n", 0),
    ("3\n", "3\n", 0),
    ("{}\n", "", 0),
    ("a: [[{x: 1}]]\n", "a = [[{ x = 1 }]]\n", 0),
    ("a: [{x: {y: 1}}]\n", "[[a]]\n[a.x]\ny = 1\n", 0),
    (
        "t: {a: 1}\nt2: {b: [{c: 1}]}\n",
        "[t]\na = 1\n\n[[t2.b]]\nc = 1\n",
        0,
    ),
    (
        "a: 1e10\nb: -0.0\nc: 100.0\nd: 12345678901\n",
        "a = 1e10\nb = -0.0\nc = 100.0\nd = 12345678901\n",
        0,
    ),
    ("a: [{}]\n", "[[a]]\n", 0),
    ("a: {b: {c: 1}}\n", "[a.b]\nc = 1\n", 0),
    (
        "a: 0x1F\nb: 0o17\nc: 1e3\nd: .5\n",
        "a = 0x1F\nb = 0o17\nc = 1e3\nd = .5\n",
        0,
    ),
    (
        "a: [{x: ~, y: 1}, {z: {w: [1, {v: 2}]}}]\n",
        "[[a]]\ny = 1\n[[a]]\n\n[a.z]\nw = [1, { v = 2 }]\n",
        0,
    ),
    ("---\na: 1\n---\nb: 2\n", "a = 1\nb = 2\n", 0),
    ("\"\": 1\n", " = 1\n", 0),
    ("\"\": {x: 1}\n", "[]\nx = 1\n", 0),
    ("\"a\\nb\"\n", "a\nb\n", 0),
    ("\"q\\\"r\"\n", "q\"r\n", 0),
    (
        "a: \"\\a\\b\\f\\r\\v\\x7f\\x1b\"\n",
        "a = \"\\a\\b\\f\\r\\v\\x7f\\x1b\"\n",
        0,
    ),
];

#[test]
fn toml_rows_match_yq_6924() -> Result<()> {
    for (input, expected, code) in ROWS {
        let (stdout, status) = run_yq_stdin(".", input, &["-o=toml"])?;
        assert_eq!(
            (stdout.as_str(), status),
            (*expected, *code),
            "-o=toml over {input:?}"
        );
    }
    Ok(())
}

/// A computed number is its own text (`+Inf`, `NaN`), as yq writes it (v4.53.3).
#[test]
fn toml_computed_non_finite_numbers_6924() -> Result<()> {
    let (stdout, status) = run_yq_stdin("{\"a\": 1 / 0, \"b\": [0 / 0]}", "a: 1\n", &["-o=toml"])?;
    assert_eq!((stdout.as_str(), status), ("a = +Inf\nb = [NaN]\n", 0));
    Ok(())
}
