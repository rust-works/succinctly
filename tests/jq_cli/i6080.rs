//! #6080: two regex readings that matched differently from Oniguruma with no error.
//!
//! - A `$` that something follows. jq's `$` matches before a final `\n` and leaves it for the next
//!   token, so `"ab\n" | match("b$\\n")` is `[1,2]`; the trimmed-view emulation of `$` had no `\n` left to
//!   consume and answered no match.
//! - An escaped letter jq does not define is the letter (`\g` matches `g`); the crate refused most of
//!   them and read `\v` as a vertical tab.
//!
//! Every row is what `/usr/bin/jq` 1.7.1 printed for the same program.

use anyhow::Result;

use crate::run_jq_stdin;

fn rows(rows: &[(&str, &str)]) -> Result<()> {
    for (program, want) in rows {
        let (stdout, code) = run_jq_stdin(program, "null", &["-c"])?;
        assert_eq!(code, 0, "`{program}` failed");
        assert_eq!(stdout.trim(), *want, "`{program}`");
    }
    Ok(())
}

#[test]
fn a_dollar_that_something_follows_leaves_the_final_newline_to_the_next_token() -> Result<()> {
    rows(&[
        (r#""ab\n" | [match("b$\\n")|[.offset,.length]]"#, "[[1,2]]"),
        (r#""ab\n" | [match("$\\n")|[.offset,.length]]"#, "[[2,1]]"),
        (r#""ab\n" | [match("b$\\s")|[.offset,.length]]"#, "[[1,2]]"),
        (r#""ab\n" | [match("b$\\n?")|[.offset,.length]]"#, "[[1,2]]"),
        (
            r#""ab\n" | [match("b$(\\n)")|[.offset,.length,.captures[0].string]]"#,
            r#"[[1,2,"\n"]]"#,
        ),
        (
            r#""ab\n" | [match("(?:b$|x)\\n")|[.offset,.length]]"#,
            "[[1,2]]",
        ),
        (
            r#""ab\n" | [match("(b$)\\n")|[.offset,.length]]"#,
            "[[1,2]]",
        ),
        (
            r#""ab\n" | [match("b$\\n";"x")|[.offset,.length]]"#,
            "[[1,2]]",
        ),
        (
            r#""ab\n" | [match("b$ \\n";"x")|[.offset,.length]]"#,
            "[[1,2]]",
        ),
        (
            r#""ab\n" | [match("b$(?#c)\\n")|[.offset,.length]]"#,
            "[[1,2]]",
        ),
        (r#""ab\n" | test("b$\\n")"#, "true"),
        (r#""ab\n" | gsub("b$\\n";"X")"#, r#""aX""#),
        (r#""ab\n" | sub("$\\n";"X")"#, r#""abX""#),
        (r#""ab\n" | [scan("b$\\n")]"#, r#"["b\n"]"#),
        (r#""ab\n" | capture("(?<x>b)$\\n")"#, r#"{"x":"b"}"#),
        (r#""éb\n" | [match("b$\\n")|[.offset,.length]]"#, "[[1,2]]"),
        // Only one final newline is the `$`'s to leave.
        (r#""ab\n\n" | [match("b$\\n")|[.offset,.length]]"#, "[]"),
        (r#""ab\n\n" | [match("$\\n")|[.offset,.length]]"#, "[[3,1]]"),
        (
            r#""a\nb\n" | [match("a$\\nb";"g")|[.offset,.length]]"#,
            "[]",
        ),
        (
            r#""a\nb\n" | [match("(?m)a$\\nb")|[.offset,.length]]"#,
            "[[0,3]]",
        ),
        // Neighbours that must not change: a `$` that ends its branch, group or pattern, and `\$`.
        (r#""ab\n" | [match("b$")|[.offset,.length]]"#, "[[1,1]]"),
        (r#""ab\n" | [match("b$|x")|[.offset,.length]]"#, "[[1,1]]"),
        (r#""ab\n" | [match("(b$)")|[.offset,.length]]"#, "[[1,1]]"),
        (r#""ab\n" | [match("(?:b$)")|[.offset,.length]]"#, "[[1,1]]"),
        (r#""ab\n" | [match("a|b$")|[.offset,.length]]"#, "[[0,1]]"),
        (
            r#""a$b\n" | [match("a\\$b$")|[.offset,.length]]"#,
            "[[0,3]]",
        ),
        (r#""a$b" | [match("[$]b")|[.offset,.length]]"#, "[[1,2]]"),
        (r#""ab" | [match("b$\\n?")|[.offset,.length]]"#, "[[1,1]]"),
    ])
}

#[test]
fn an_escaped_letter_jq_does_not_define_is_the_letter() -> Result<()> {
    rows(&[
        (r#""g" | [match("\\g")|.string]"#, r#"["g"]"#),
        (r#""h" | [match("\\h")|.string]"#, r#"["h"]"#),
        (r#""f" | [match("\\h")|.string]"#, "[]"),
        (r#""v" | [match("\\v")|.string]"#, r#"["v"]"#),
        (r#""gv" | [match("[\\g\\v]";"g")|.string]"#, r#"["g","v"]"#),
        (r#""ghi" | [match("\\g\\h\\i")|.string]"#, r#"["ghi"]"#),
        (r#""k" | test("\\k")"#, "true"),
        (r#""T" | test("\\T")"#, "true"),
        (r#""E" | test("\\E")"#, "true"),
        (r#""q" | [match("(\\q)")|.captures[0].string]"#, r#"["q"]"#),
        (r#""h h" | [match("\\h\\s\\h";"x")|.string]"#, r#"["h h"]"#),
        (r#""a1" | [match("\\d\\i?")|.string]"#, r#"["1"]"#),
        (r#""gg" | [match("\\g{2}")|.string]"#, r#"["gg"]"#),
        (r#""abg1" | test("(ab)\\g1")"#, "true"),
        (r#""abab" | test("(ab)\\g1")"#, "false"),
        (r#""g" | sub("\\g";"X")"#, r#""X""#),
        (r#""g" | split("\\g";null)"#, r#"["",""]"#),
        (r#""u00e9" | test("\\u00e9")"#, "true"),
        // Neighbours that must not change: `\k<n>` is a back-reference, `\\g` an escaped backslash
        // and `g`, and the escapes that are defined keep their meaning.
        (r#""abab" | test("(ab)\\k<1>")"#, "true"),
        (r#""a\\g" | [match("a\\\\g")|.string]"#, r#"["a\\g"]"#),
        (r#""a1" | [match("\\d")|.string]"#, r#"["1"]"#),
        (r#""ab" | test("a\\bb")"#, "false"),
    ])
}
