//! #7528: a pattern nests groups at most 511 deep, as in jq 1.8.2 (GHSA-f946-j5j2-4w5m, which gave
//! Oniguruma a parse depth limit); from 512 it is `Regex failure: parse depth limit over`. jq 1.7.1
//! accepted up to 2,047 and the `regex` crate stopped at 250 nested AST nodes, which counts every
//! concatenation, alternation and repeat between two groups where jq counts groups only. Every
//! expectation was captured from jq 1.8.2 (`/opt/homebrew/bin/jq`) for the same program; 251 to 511
//! are answered by jq 1.7.1 too. The patterns run through the debug binary, so a depth that overran
//! the native stack of a debug build would fail here.

use anyhow::Result;

use crate::run_jq_null;

/// (builtin on `$p`, text opening a group level, text closing it, groups, expected output); the
/// pattern is `open * depth + "a" + close * depth`.
const ROWS: &[(&str, &str, &str, usize, &str)] = &[
    ("test($p)", "(", ")", 250, "true"),
    ("test($p)", "(", ")", 251, "true"),
    ("test($p)", "(", ")", 400, "true"),
    ("test($p)", "(", ")", 511, "true"),
    (
        "test($p)",
        "(",
        ")",
        512,
        "\"Regex failure: parse depth limit over\"",
    ),
    (
        "test($p)",
        "(",
        ")",
        513,
        "\"Regex failure: parse depth limit over\"",
    ),
    (
        "test($p)",
        "(",
        ")",
        2047,
        "\"Regex failure: parse depth limit over\"",
    ),
    ("test($p)", "(?:", ")", 250, "true"),
    ("test($p)", "(?:", ")", 251, "true"),
    ("test($p)", "(?:", ")", 400, "true"),
    ("test($p)", "(?:", ")", 511, "true"),
    (
        "test($p)",
        "(?:",
        ")",
        512,
        "\"Regex failure: parse depth limit over\"",
    ),
    (
        "test($p)",
        "(?:",
        ")",
        513,
        "\"Regex failure: parse depth limit over\"",
    ),
    (
        "test($p)",
        "(?:",
        ")",
        2047,
        "\"Regex failure: parse depth limit over\"",
    ),
    ("test($p)", "x(", ")y", 250, "false"),
    ("test($p)", "x(", ")y", 251, "false"),
    ("test($p)", "x(", ")y", 400, "false"),
    ("test($p)", "x(", ")y", 511, "false"),
    (
        "test($p)",
        "x(",
        ")y",
        512,
        "\"Regex failure: parse depth limit over\"",
    ),
    (
        "test($p)",
        "x(",
        ")y",
        513,
        "\"Regex failure: parse depth limit over\"",
    ),
    (
        "test($p)",
        "x(",
        ")y",
        2047,
        "\"Regex failure: parse depth limit over\"",
    ),
    ("test($p)", "(x|", ")", 250, "true"),
    ("test($p)", "(x|", ")", 251, "true"),
    ("test($p)", "(x|", ")", 400, "true"),
    ("test($p)", "(x|", ")", 511, "true"),
    (
        "test($p)",
        "(x|",
        ")",
        512,
        "\"Regex failure: parse depth limit over\"",
    ),
    (
        "test($p)",
        "(x|",
        ")",
        513,
        "\"Regex failure: parse depth limit over\"",
    ),
    (
        "test($p)",
        "(x|",
        ")",
        2047,
        "\"Regex failure: parse depth limit over\"",
    ),
    ("test($p)", "(", ")*", 250, "true"),
    ("test($p)", "(", ")*", 251, "true"),
    ("test($p)", "(", ")*", 400, "true"),
    ("test($p)", "(", ")*", 511, "true"),
    (
        "test($p)",
        "(",
        ")*",
        512,
        "\"Regex failure: parse depth limit over\"",
    ),
    (
        "test($p)",
        "(",
        ")*",
        513,
        "\"Regex failure: parse depth limit over\"",
    ),
    (
        "test($p)",
        "(",
        ")*",
        2047,
        "\"Regex failure: parse depth limit over\"",
    ),
    ("test($p)", "(", ")*?", 250, "true"),
    ("test($p)", "(", ")*?", 251, "true"),
    ("test($p)", "(", ")*?", 400, "true"),
    ("test($p)", "(", ")*?", 511, "true"),
    (
        "test($p)",
        "(",
        ")*?",
        512,
        "\"Regex failure: parse depth limit over\"",
    ),
    (
        "test($p)",
        "(",
        ")*?",
        513,
        "\"Regex failure: parse depth limit over\"",
    ),
    (
        "test($p)",
        "(",
        ")*?",
        2047,
        "\"Regex failure: parse depth limit over\"",
    ),
    ("test($p)", "(?i)(", ")", 250, "true"),
    ("test($p)", "(?i)(", ")", 251, "true"),
    ("test($p)", "(?i)(", ")", 400, "true"),
    ("test($p)", "(?i)(", ")", 511, "true"),
    (
        "test($p)",
        "(?i)(",
        ")",
        512,
        "\"Regex failure: parse depth limit over\"",
    ),
    (
        "test($p)",
        "(?i)(",
        ")",
        513,
        "\"Regex failure: parse depth limit over\"",
    ),
    (
        "test($p)",
        "(?i)(",
        ")",
        2047,
        "\"Regex failure: parse depth limit over\"",
    ),
    ("test($p; \"ix\")", "(", ")", 250, "true"),
    ("test($p; \"ix\")", "(", ")", 251, "true"),
    ("test($p; \"ix\")", "(", ")", 400, "true"),
    ("test($p; \"ix\")", "(", ")", 511, "true"),
    (
        "test($p; \"ix\")",
        "(",
        ")",
        512,
        "\"Regex failure: parse depth limit over\"",
    ),
    (
        "test($p; \"ix\")",
        "(",
        ")",
        513,
        "\"Regex failure: parse depth limit over\"",
    ),
    (
        "test($p; \"ix\")",
        "(",
        ")",
        2047,
        "\"Regex failure: parse depth limit over\"",
    ),
    (
        "[match($p; \"gi\")|.captures|length]",
        "(",
        ")",
        250,
        "[250]",
    ),
    (
        "[match($p; \"gi\")|.captures|length]",
        "(",
        ")",
        251,
        "[251]",
    ),
    (
        "[match($p; \"gi\")|.captures|length]",
        "(",
        ")",
        400,
        "[400]",
    ),
    (
        "[match($p; \"gi\")|.captures|length]",
        "(",
        ")",
        511,
        "[511]",
    ),
    (
        "[match($p; \"gi\")|.captures|length]",
        "(",
        ")",
        512,
        "\"Regex failure: parse depth limit over\"",
    ),
    (
        "[match($p; \"gi\")|.captures|length]",
        "(",
        ")",
        513,
        "\"Regex failure: parse depth limit over\"",
    ),
    (
        "[match($p; \"gi\")|.captures|length]",
        "(",
        ")",
        2047,
        "\"Regex failure: parse depth limit over\"",
    ),
    ("capture($p)", "(", ")", 250, "{}"),
    ("capture($p)", "(", ")", 251, "{}"),
    ("capture($p)", "(", ")", 400, "{}"),
    ("capture($p)", "(", ")", 511, "{}"),
    (
        "capture($p)",
        "(",
        ")",
        512,
        "\"Regex failure: parse depth limit over\"",
    ),
    (
        "capture($p)",
        "(",
        ")",
        513,
        "\"Regex failure: parse depth limit over\"",
    ),
    (
        "capture($p)",
        "(",
        ")",
        2047,
        "\"Regex failure: parse depth limit over\"",
    ),
    ("[scan($p)]|length", "(", ")", 250, "1"),
    ("[scan($p)]|length", "(", ")", 251, "1"),
    ("[scan($p)]|length", "(", ")", 400, "1"),
    ("[scan($p)]|length", "(", ")", 511, "1"),
    (
        "[scan($p)]|length",
        "(",
        ")",
        512,
        "\"Regex failure: parse depth limit over\"",
    ),
    (
        "[scan($p)]|length",
        "(",
        ")",
        513,
        "\"Regex failure: parse depth limit over\"",
    ),
    (
        "[scan($p)]|length",
        "(",
        ")",
        2047,
        "\"Regex failure: parse depth limit over\"",
    ),
    ("sub($p; \"X\")", "(", ")", 250, "\"X\""),
    ("sub($p; \"X\")", "(", ")", 251, "\"X\""),
    ("sub($p; \"X\")", "(", ")", 400, "\"X\""),
    ("sub($p; \"X\")", "(", ")", 511, "\"X\""),
    (
        "sub($p; \"X\")",
        "(",
        ")",
        512,
        "\"Regex failure: parse depth limit over\"",
    ),
    (
        "sub($p; \"X\")",
        "(",
        ")",
        513,
        "\"Regex failure: parse depth limit over\"",
    ),
    (
        "sub($p; \"X\")",
        "(",
        ")",
        2047,
        "\"Regex failure: parse depth limit over\"",
    ),
    ("gsub($p; \"X\")", "(", ")", 250, "\"X\""),
    ("gsub($p; \"X\")", "(", ")", 251, "\"X\""),
    ("gsub($p; \"X\")", "(", ")", 400, "\"X\""),
    ("gsub($p; \"X\")", "(", ")", 511, "\"X\""),
    (
        "gsub($p; \"X\")",
        "(",
        ")",
        512,
        "\"Regex failure: parse depth limit over\"",
    ),
    (
        "gsub($p; \"X\")",
        "(",
        ")",
        513,
        "\"Regex failure: parse depth limit over\"",
    ),
    (
        "gsub($p; \"X\")",
        "(",
        ")",
        2047,
        "\"Regex failure: parse depth limit over\"",
    ),
    ("split($p; null)|length", "(", ")", 250, "2"),
    ("split($p; null)|length", "(", ")", 251, "2"),
    ("split($p; null)|length", "(", ")", 400, "2"),
    ("split($p; null)|length", "(", ")", 511, "2"),
    (
        "split($p; null)|length",
        "(",
        ")",
        512,
        "\"Regex failure: parse depth limit over\"",
    ),
    (
        "split($p; null)|length",
        "(",
        ")",
        513,
        "\"Regex failure: parse depth limit over\"",
    ),
    (
        "split($p; null)|length",
        "(",
        ")",
        2047,
        "\"Regex failure: parse depth limit over\"",
    ),
    ("[splits($p)]|length", "(", ")", 250, "2"),
    ("[splits($p)]|length", "(", ")", 251, "2"),
    ("[splits($p)]|length", "(", ")", 400, "2"),
    ("[splits($p)]|length", "(", ")", 511, "2"),
    (
        "[splits($p)]|length",
        "(",
        ")",
        512,
        "\"Regex failure: parse depth limit over\"",
    ),
    (
        "[splits($p)]|length",
        "(",
        ")",
        513,
        "\"Regex failure: parse depth limit over\"",
    ),
    (
        "[splits($p)]|length",
        "(",
        ")",
        2047,
        "\"Regex failure: parse depth limit over\"",
    ),
];

#[test]
fn test_nested_group_limit_is_511_as_jq_182_7528() -> Result<()> {
    for (builtin, open, close, depth, expected) in ROWS {
        let filter = format!(
            "\"a\" | (({open:?} * {depth}) + \"a\" + ({close:?} * {depth})) as $p | try ({builtin}) catch ."
        );
        let (stdout, code) = run_jq_null(&filter, &["-c"])?;
        assert_eq!(
            stdout,
            format!("{expected}\n"),
            "{builtin} {open}..{close} x {depth}"
        );
        assert_eq!(code, 0, "{builtin} {open}..{close} x {depth}");
    }
    Ok(())
}

/// Uncaught, the refusal is a runtime error with the exit status of one.
#[test]
fn test_nested_group_refusal_is_an_error_7528() -> Result<()> {
    let (stdout, code) = run_jq_null(r#""a" | test(("(" * 512) + "a" + (")" * 512))"#, &["-c"])?;
    assert_eq!(stdout, "");
    assert_eq!(code, 5);
    Ok(())
}
