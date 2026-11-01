//! #5414: `-C` and `-0` no longer send a top-level `,` of navigation branches back to the DOM
//! route, which rebuilt each result as an `IndexMap` and collapsed a repeated mapping key that
//! yq prints (#5283 fixed the plain case). Expected bytes are yq v4.53.3's.

use anyhow::Result;

use crate::run_yq_stdin;

const DUP: &str = "a: 1\na: 2\nb: 3\n";

/// Strips SGR escape sequences (`ESC [ ... m`).
fn strip_ansi(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for d in chars.by_ref() {
                if d == 'm' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[test]
fn colored_comma_keeps_a_repeated_key() -> Result<()> {
    let (plain, _) = run_yq_stdin("., .b", DUP, &[])?;
    assert_eq!(plain, "a: 1\na: 2\nb: 3\n3\n");
    let (colored, code) = run_yq_stdin("., .b", DUP, &["-C"])?;
    assert_eq!(code, 0);
    assert_eq!(strip_ansi(&colored), plain);
    Ok(())
}

#[test]
fn colored_json_comma_keeps_a_repeated_key() -> Result<()> {
    let (plain, _) = run_yq_stdin("., .b", DUP, &["-o=json", "-I0"])?;
    assert_eq!(plain, "{\"a\":1,\"a\":2,\"b\":3}\n3\n");
    let (colored, code) = run_yq_stdin("., .b", DUP, &["-C", "-o=json", "-I0"])?;
    assert_eq!(code, 0);
    assert_eq!(strip_ansi(&colored), plain);
    Ok(())
}

#[test]
fn nul_terminated_comma_keeps_a_repeated_key() -> Result<()> {
    let (out, code) = run_yq_stdin("., .b", DUP, &["-0"])?;
    assert_eq!(code, 0);
    assert_eq!(out, "a: 1\na: 2\nb: 3\x003\x00");
    let (out, _) = run_yq_stdin("., .b", DUP, &["-0", "-o=json", "-I0"])?;
    assert_eq!(out, "{\"a\":1,\"a\":2,\"b\":3}\x003\x00");
    Ok(())
}

#[test]
fn nul_in_a_later_branch_prints_the_earlier_one_then_fails() -> Result<()> {
    let input = "a: 1\nb: \"x\\0y\"\n";
    let (out, code) = run_yq_stdin(".a, .b", input, &["-0"])?;
    assert_eq!(out, "1\x00");
    assert_eq!(code, 1);
    let (out, code) = run_yq_stdin(".b, .a", input, &["-0"])?;
    assert_eq!(out, "");
    assert_eq!(code, 1);
    Ok(())
}
