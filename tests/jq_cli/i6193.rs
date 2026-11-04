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
    (r"[^^]b$", Some("[]")),
    (r"(?m)[^^]|a$", Some("[[0,1],[1,1],[2,1],[4,1]]")),
    (r"(?:[^^]a)*b", Some("[[0,3]]")),
    (r"([^^])+b", Some("[[0,3]]")),
    (r"(?:[^^]){0}b", Some("[[2,1]]")),
    (r"(?=a)([^^])", Some("[[0,1],[1,1]]")),
    (r"[^^]x$", Some("[]")),
    (r"^[^^]*$", Some("[]")),
    (r"(?<=a)[^^]", Some("[[1,1],[2,1]]")),
    (r"(?:[^^]|)*b$", Some("[]")),
    (r"[^^]]$", Some("[]")),
    (r"([^^]]|b)$", Some("[]")),
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

/// The same classes where two other scans read them: the check for a repeated group that is an
/// assertion (`(?:[^^]|\b)*` is refused in jq), and the `$` rewrite of the second engine, which a
/// class left open would skip (`(?=a)[^^]$` matches before a final newline). Each row has its own
/// text.
const SCAN_ROWS: &[(&str, &str, Option<&str>)] = &[
    ("aab^]", r"(?:[^^]|\b)*", None),
    ("aab^]", r"(?:\b|[^^])*b", None),
    (
        "aab^]",
        r"(?:[^^]\b)*",
        Some("[[0,0],[1,0],[2,1],[3,0],[4,0],[5,0]]"),
    ),
    ("aab^]", r"(?:[^^]|a)*", Some("[[0,3],[3,0],[4,1],[5,0]]")),
    ("a\n", r"(?=a)[^^]$", Some("[[0,1]]")),
    ("a\n", r"(?=a)[^^]$\n", Some("[[0,2]]")),
    ("ab\n", r"(?<=a)[^^]$", Some("[[1,1]]")),
    ("a\n", r"(?=a)[^^]\Z", Some("[[0,1]]")),
    ("a\n", r"(?=a)[^^]\z", Some("[]")),
    ("a\n", r"(?=a)[^^^]$", Some("[[0,1]]")),
    ("aab^]", r"([[:alpha:]^]){0}b", Some("[[2,1]]")),
    ("aab^]", r"([^[:alpha:]]){0}b", Some("[[2,1]]")),
    ("aab^]", r"(?:[^^]?(?#x))*b", Some("[[0,3]]")),
    ("aab^]", r"(?:a|[[:alpha:]^]?)*(?>b)", Some("[[0,3]]")),
    ("aab^]", r"(?:[^^]??(?!c))*?b", Some("[[0,3]]")),
    ("aab^]", r"(?:[^^]?\K)*b", Some("[[2,1]]")),
];

#[test]
fn test_a_second_caret_ends_the_class_start_in_every_scan_6193() -> Result<()> {
    for (text, pattern, want) in SCAN_ROWS {
        let args = [
            "-c",
            "--arg",
            "p",
            pattern,
            r#"[match($p;"g")|[.offset,.length]]"#,
        ];
        let input = format!("{text:?}");
        let (stdout, stderr, code) = run_jq_full(&args, Some(&input))?;
        match want {
            Some(want) => assert_eq!(
                (stdout.trim_end(), code),
                (*want, 0),
                "{pattern} on {text:?}: {stderr}"
            ),
            None => {
                assert_eq!((stdout.as_str(), code), ("", 5), "{pattern}: {stderr}");
                assert!(stderr.contains("invalid regex"), "{pattern}: {stderr}");
            }
        }
    }
    Ok(())
}
