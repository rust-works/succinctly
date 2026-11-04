//! #6133: Oniguruma lets an option run repeat `-` (a later `-` is a no-op, and letters after it are
//! still turned off) and end on one (`(?-)`, `(?i-:...)`); the `regex` crate refuses both. Every
//! row was captured from jq 1.7.1 (`/usr/bin/jq -nc`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_repeated_or_trailing_minus_in_an_option_run_is_read_as_jq_does_6133() -> Result<()> {
    for (filter, expected) in [
        (r#""a" | test("(?-)a")"#, "true"),
        (r#""a" | test("(?--i)a")"#, "true"),
        (r#""A" | test("(?--i)a")"#, "false"),
        (r#""A" | test("(?i-m-s)a")"#, "true"),
        (r#""A" | test("(?i--)a")"#, "true"),
        (r#""A" | test("(?i-:a)")"#, "true"),
        (r#""A" | test("(?-:a)")"#, "false"),
        (r#""a" | test("(?-:a)b")"#, "false"),
        (r#""A" | test("(?i--m)a")"#, "true"),
        (r#""a b" | test("(?x-)a b")"#, "false"),
        (r#""ab" | test("(?x-)a b")"#, "true"),
        (r#""ab" | test("(?x-i-x)a b")"#, "false"),
        (r#""a b" | test("(?x-i-x)a b")"#, "true"),
        (r#""a b" | test("(?i-m-x)a b")"#, "true"),
        (r#""ab" | test("(?i-m-x)a b")"#, "false"),
        (r#""ab" | test("(?--x:a b)")"#, "false"),
        (r#""ab" | test("(?x--:a b)")"#, "true"),
        (r#""a\nb" | test("(?s-)a.b")"#, "true"),
        (r#""a\nb" | test("(?-s-m)a.b")"#, "false"),
        (r#""xa" | test("x(?-)a")"#, "true"),
        (r#""xA" | test("x(?i-:a)")"#, "true"),
        (r#""xA" | test("x(?i-:a)|b")"#, "true"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(stdout.trim_end(), expected, "{filter}: stderr={stderr:?}");
        assert_eq!(code, 0, "{filter}");
    }
    Ok(())
}

/// The option run is rewritten for every regex builtin, not only `test`.
#[test]
fn the_rewrite_reaches_match_sub_and_scan_6133() -> Result<()> {
    for (filter, expected) in [
        (
            r#""aA" | [match("(?i-m-s)a";"g") | .string]"#,
            r#"["a","A"]"#,
        ),
        (r#""aA" | sub("(?--i)a";"X")"#, r#""XA""#),
        (r#""aAa" | [scan("(?i-:a)")]"#, r#"["a","A","a"]"#),
        (r#""ab" | capture("(?-)(?<x>a)")"#, r#"{"x":"a"}"#),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(stdout.trim_end(), expected, "{filter}: stderr={stderr:?}");
        assert_eq!(code, 0, "{filter}");
    }
    Ok(())
}
