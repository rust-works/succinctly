//! #6197: Oniguruma refuses a capture group or a lookahead inside a negative look-behind, and a
//! lookahead or a negative look-behind inside a positive one, at any depth (`invalid pattern in
//! look-behind`); the `regex` and `fancy-regex` engines accept all of them. Every row was checked
//! against jq 1.7.1 (`/usr/bin/jq -nc`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn what_jq_refuses_inside_a_look_behind_is_refused_6197() -> Result<()> {
    for pattern in [
        "(?<!(a))b",
        "(?<!(?<n>a))b",
        "(?<!(?:(a)))b",
        "(?<!(?'n'a))b",
        "(?<!a(b))c",
        "(?<!(?i)(a))b",
        "(?<!(?i:(a)))b",
        "(?<!(?:a|(b)))c",
        "(?<!(?<!(a)))z",
        "(?<!(?=a)b)c",
        "(?<!(?!a)b)c",
        "(?<!(?=(a)))b",
        "(?<=(?<!a)b)",
        "(?<=(?<!a))b",
        "(?<=(?=a)b)c",
        "(?<=(?!a)b)c",
        "(?<=a(?<!b))c",
        "(?<=(?<=(?<!a)))z",
        "(?=(?<=a(?=b)))",
        "(?<!a)(?<!(b))c",
        "(?<![(](a))b",
    ] {
        let filter = format!(
            r#""cb" | [match({};"g") | .string]"#,
            serde_json::to_string(pattern)?
        );
        let (stdout, stderr, code) = run_jq_full(&["-nc", &filter], None)?;
        assert_eq!(stdout.trim_end(), "", "{pattern}");
        assert_ne!(code, 0, "{pattern}");
        assert!(stderr.contains("look-behind"), "{pattern}: {stderr:?}");
    }
    Ok(())
}

/// The neighbouring shapes jq accepts keep answering.
#[test]
fn what_jq_accepts_inside_a_look_behind_still_answers_6197() -> Result<()> {
    for (filter, expected) in [
        (r#""cb ab" | [match("(?<=(a))b";"g")|.string]"#, r#"["b"]"#),
        (
            r#""cb ab" | [match("(?<=(?<n>a))b";"g")|.string]"#,
            r#"["b"]"#,
        ),
        (
            r#""cb ab" | [match("(?<=(a)|c)b";"g")|.string]"#,
            r#"["b","b"]"#,
        ),
        (
            r#""cb ab" | [match("(?<=(?<=a)b)";"g")|.string]"#,
            r#"[""]"#,
        ),
        (
            r#""cb ab" | [match("(?<!(?<!a)b)c";"g")|.string]"#,
            r#"["c"]"#,
        ),
        (
            r#""cb ab" | [match("(?<!(?<=a)b)c";"g")|.string]"#,
            r#"["c"]"#,
        ),
        (r#""cb ab" | [match("(?<=(?>a)b)c";"g")|.string]"#, "[]"),
        (
            r#""cb ab" | [match("(?<!(?>a)b)c";"g")|.string]"#,
            r#"["c"]"#,
        ),
        (r#""cb ab" | [match("(?=(a))b";"g")|.string]"#, "[]"),
        (
            r#""cb ab" | [match("(?!(a))b";"g")|.string]"#,
            r#"["b","b"]"#,
        ),
        (r#""cb ab" | [match("(?<!x(?<!y))z";"g")|.string]"#, "[]"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(stdout.trim_end(), expected, "{filter}: stderr={stderr:?}");
        assert_eq!(code, 0, "{filter}");
    }
    Ok(())
}
