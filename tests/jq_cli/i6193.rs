//! #6193: only the first `^` after a class's `[` negates it. The scan kept the class at its start
//! through every `^`, so in `[^^]z[]` the `]` after the second `^` was read as a literal and the
//! class ran on into what followed, and an unterminated class after a `[^^]` was accepted where jq
//! refuses it. Rows are live jq 1.7.1 captures: the offsets and lengths of every match in one text,
//! or `None` for an error (exit 5; the message is the engine's own).

use anyhow::Result;

use crate::run_jq_full;

const TEXT: &str = r#""aab^]""#;

const ROWS: &[(&str, Option<&str>)] = &[
    (r"[^^]z[]", None),
    (r"[^^]z[", None),
    (r"[^^][", None),
    (r"[^^]", Some("[[0,1],[1,1],[2,1],[4,1]]")),
    (r"[^]a]", Some("[[2,1],[3,1]]")),
    (r"[^^^]", Some("[[0,1],[1,1],[2,1],[4,1]]")),
    (r"[^^]]", Some("[]")),
    (r"[^]]", Some("[[0,1],[1,1],[2,1],[3,1]]")),
    (r"[^^a]", Some("[[2,1],[4,1]]")),
    (r"[^^]^", Some("[]")),
    (r"[^^\]]", Some("[[0,1],[1,1],[2,1]]")),
    (r"(?:[^^]|b)+", Some("[[0,3],[4,1]]")),
    (r"[^^]{2}a", Some("[]")),
    (r"([^^]){0}b", Some("[[2,1]]")),
    (r"([^^]]){0}b", Some("[[2,1]]")),
    (r"(?=[^^]])a", Some("[]")),
    (r"(?=[^^])a", Some("[[0,1],[1,1]]")),
    (r"(?<=[^^])b", Some("[[2,1]]")),
    (r"(?>[^^]])b", Some("[]")),
    (r"([^^]|a)(?:b){0}", Some("[[0,1],[1,1],[2,1],[4,1]]")),
    (r"[^^]*+b", Some("[]")),
    (r"[^^]*+z[]", None),
];

#[test]
fn test_only_the_first_caret_of_a_class_negates_it_6193() -> Result<()> {
    for (pattern, want) in ROWS {
        let args = [
            "-c",
            "--arg",
            "p",
            pattern,
            r#"[match($p;"g")|[.offset,.length]]"#,
        ];
        let (stdout, stderr, code) = run_jq_full(&args, Some(TEXT))?;
        match want {
            Some(want) => assert_eq!((stdout.trim_end(), code), (*want, 0), "{pattern}: {stderr}"),
            None => {
                assert_eq!((stdout.as_str(), code), ("", 5), "{pattern}: {stderr}");
                assert!(stderr.contains("invalid regex"), "{pattern}: {stderr}");
            }
        }
    }
    Ok(())
}
