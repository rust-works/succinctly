//! #6196: Oniguruma compares a backreference under the options in force where the backreference
//! stands, not those its group matched under. `"aA" | test("((?i)a)\\1")` is `false` (the `(?i)`
//! ended with its group) and `test("(?i)((?-i)a)\\1")` is `true`. The `fancy-regex` route, which a
//! backreference takes, applied the option in force where the group ended. Each backreference is
//! now written as `(?i:\1)` or `(?-i:\1)` for the mode in force where it stands, when the pattern
//! holds an inline `i` option. Every row was captured from jq 1.7.1 (`/usr/bin/jq -n`).

use anyhow::Result;

use crate::run_jq_full;

fn answer(pattern: &str, input: &str, flags: &str) -> Result<String> {
    let program = format!(
        r#""{input}" | [match("{}";"{flags}") | .string]"#,
        pattern.replace('\\', "\\\\")
    );
    let (stdout, stderr, code) = run_jq_full(&["-nc", &program], None)?;
    assert_eq!(code, 0, "{pattern} on {input}: stderr={stderr:?}");
    Ok(stdout.trim_end().to_string())
}

#[test]
fn test_a_backreference_compares_under_the_option_in_force_where_it_stands_6196() -> Result<()> {
    for (pattern, input, flags, expected) in [
        (r"((?i)a)\1", "aA", "g", r"[]"),
        (r"(((?i)a))\1", "aA", "g", r"[]"),
        (r"((?i)(a))\1", "aA", "g", r"[]"),
        (r"((?i)a|b)\1", "aA", "g", r"[]"),
        (r"((?ii)a)\1", "aA", "g", r"[]"),
        (r"(?<n>(?i)a)\k<n>", "aA", "g", r"[]"),
        (r"(?<n>(?i)a)\k<1>", "aA", "g", r"[]"),
        (r"(?i)((?-i)a)\1", "aA", "g", r#"["aA"]"#),
        (r"(?i)((?i-i)a)\1", "aA", "g", r#"["aA"]"#),
        (r"(?i)(?i)((?-i)a)\1", "aA", "g", r#"["aA"]"#),
        (r"(?-i)((?i)a)\1", "aA", "g", r"[]"),
        (r"(a)(?=(?i)\1)\1", "aA", "g", r"[]"),
        (r"(?<=(?i)(a))\1", "aA", "g", r"[]"),
        (r"(a)(?>(?i)\1)\1", "aAa", "g", r#"["aAa"]"#),
        (r"((?i)a|(?-i)b)\1", "aA", "gi", r#"["aA"]"#),
        (r"((?i)a)\1", "aA", "gi", r#"["aA"]"#),
        (r"(?i)(a)(?-i)\1", "aA", "g", r"[]"),
        (r"(a)(?i)\1(?-i)\1", "aAa", "g", r#"["aAa"]"#),
        (r"(?i:(a))\1", "aA", "g", r"[]"),
        (r"(?i:(a)\1)", "aA", "g", r#"["aA"]"#),
        (r"((?i:a))\1", "aA", "g", r"[]"),
        (r"(?:(?i)(a))\1", "aA", "g", r"[]"),
        (
            r"(a)(b)(c)(d)(e)(f)(g)(h)(i)((?i)j)\10",
            "abcdefghijJ",
            "g",
            r"[]",
        ),
        (r"(a)\Q(?i)\E\1", "a(?i)a", "g", r#"["a(?i)a"]"#),
        (r"(a)(?#i)\1", "aA", "g", r"[]"),
        (r"(a)(?i)(?#)\1", "aA", "g", r#"["aA"]"#),
    ] {
        assert_eq!(
            answer(pattern, input, flags)?,
            expected,
            "{pattern} on {input} ({flags})"
        );
    }
    Ok(())
}
