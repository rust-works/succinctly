//! #5550: a `(?P` inside a comment is text, not a Python-style group. jq ignores a `(?#...)`
//! comment, and a `#` comment where the extended mode is on (the `x` flag or an inline
//! `(?x)`, `(?x:...)`, `(?-x)`, followed group by group); `succinctly jq` refused the pattern
//! (`undefined group option`) because the pre-pass read the raw text. Every row was captured from
//! jq 1.7.1 (`/usr/bin/jq -nc`, input `"ab"`); `REFUSED` is an error exit.

use anyhow::Result;

use crate::run_jq_full;

/// (pattern, flags, jq's answer)
const ROWS: &[(&str, Option<&str>, &str)] = &[
    ("ab # (?P\n", Some("x"), "true"),
    ("(?x) ab # (?P<n>\n", None, "true"),
    ("(?#(?P)ab", None, "true"),
    ("(?<=a)b # (?P\n", Some("x"), "true"),
    ("(?<=a)(?#(?P)b", None, "true"),
    ("(?x:ab # (?P\n)", None, "true"),
    ("(?x:a # (?P<n>\n)b", None, "true"),
    ("a(?x) # (?P\nb", None, "true"),
    ("(a(?x) # (?P\n)b", None, "true"),
    ("(?ix: a # (?P\n b)", None, "true"),
    ("ab # [\n", Some("x"), "true"),
    ("ab # \\<\n", Some("x"), "true"),
    ("a(?#[)b", None, "true"),
    ("a(?#\\<)b", None, "true"),
    // A `#` is a literal without the extended mode, and then the `(?P` is a real group.
    ("ab # (?P\n", None, "REFUSED"),
    ("(?x:a) # (?P\n", None, "REFUSED"),
    ("(?x)(?-x)ab # (?P\n", None, "REFUSED"),
    ("(?x:(?-x:a # (?P\n))b", None, "REFUSED"),
    ("(?-x)ab # (?P\n", Some("x"), "REFUSED"),
    ("(?:(?x)a # (?P\n)b # (?P\n", None, "REFUSED"),
    // Real groups next to a comment, an escaped `#`, a `#` in a class.
    ("(?x)a#(?P\n(?P<n>b)", None, "REFUSED"),
    ("(?x)\\#(?P<n>a)", None, "REFUSED"),
    ("(?P<n>ab)", None, "REFUSED"),
    ("(?x)[#(?P]ab", None, "false"),
];

#[test]
fn test_a_p_group_in_a_comment_is_text_5550() -> Result<()> {
    for (pattern, flags, expected) in ROWS {
        let escaped = pattern
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n");
        let flag_arg = flags.map_or(String::new(), |f| format!(";\"{f}\""));
        let filter = format!(r#""ab" | test("{escaped}"{flag_arg})"#);
        let (stdout, stderr, code) = run_jq_full(&["-nc", &filter], None)?;
        if *expected == "REFUSED" {
            assert_eq!(
                code, 5,
                "{pattern:?} {flags:?}: stdout={stdout:?} stderr={stderr:?}"
            );
            assert!(
                stderr.contains("undefined group option"),
                "{pattern:?} {flags:?}: {stderr:?}"
            );
        } else {
            assert_eq!(
                stdout.trim_end(),
                *expected,
                "{pattern:?} {flags:?}: stderr={stderr:?}"
            );
            assert_eq!(code, 0, "{pattern:?} {flags:?}");
        }
    }
    Ok(())
}
