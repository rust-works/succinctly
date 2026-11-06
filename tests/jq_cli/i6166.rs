//! #6166: a lazy repeat that may stop at once (`*?`, `??`, `{0,n}?`) of a group with a `\G` in its
//! body (`(?:\K\G)*?b`) found nothing where jq finds the `b` after it, because `fancy-regex` 0.20.0
//! gives such a `\G` an early exit that ends the search where the repeat could still stop at once.
//! A never-matching way, `|(?!)`, is added to the group's body for the engine. Every row was
//! captured from jq 1.7.1 (`/usr/bin/jq -n`).

use anyhow::Result;

use crate::run_jq_full;

/// `SUBJECT | [match(PATTERN; "g") | [.offset, .length, .string]]`, compact.
fn matches(subject: &str, pattern: &str) -> Result<(String, String, i32)> {
    let filter = format!(
        r#""{subject}" | [match("{}"; "g") | [.offset, .length, .string]]"#,
        pattern.replace('\\', "\\\\").replace('"', "\\\"")
    );
    run_jq_full(&["-nc", &filter], None)
}

#[test]
fn test_a_lazy_repeat_of_a_g_k_body_finds_the_later_match_6166() -> Result<()> {
    for (pattern, on_cab, on_cabb) in [
        (r"(?:\K\G)*?b", r#"[[2,1,"b"]]"#, r#"[[2,1,"b"],[3,1,"b"]]"#),
        (r"(?:\G\K)??b", r#"[[2,1,"b"]]"#, r#"[[2,1,"b"],[3,1,"b"]]"#),
        (r"(?>\G)*?b", r#"[[2,1,"b"]]"#, r#"[[2,1,"b"],[3,1,"b"]]"#),
        (r"(?>\K\G)*?b", r#"[[2,1,"b"]]"#, r#"[[2,1,"b"],[3,1,"b"]]"#),
        (r"(?:\G\G)*?b", r#"[[2,1,"b"]]"#, r#"[[2,1,"b"],[3,1,"b"]]"#),
        (
            r"(?i:\K\G)*?b",
            r#"[[2,1,"b"]]"#,
            r#"[[2,1,"b"],[3,1,"b"]]"#,
        ),
        (
            r"(?x: \K \G )*?b",
            r#"[[2,1,"b"]]"#,
            r#"[[2,1,"b"],[3,1,"b"]]"#,
        ),
        (
            r"(?:\K\G){0,2}?b",
            r#"[[2,1,"b"]]"#,
            r#"[[2,1,"b"],[3,1,"b"]]"#,
        ),
        (
            r"(?:\K(?:\G))*?b",
            r#"[[2,1,"b"]]"#,
            r#"[[2,1,"b"],[3,1,"b"]]"#,
        ),
        (
            r"(?:\K\G){0}?b",
            r#"[[2,1,"b"]]"#,
            r#"[[2,1,"b"],[3,1,"b"]]"#,
        ),
        (r"a(?:\K\G)*?b", r#"[[1,2,"ab"]]"#, r#"[[1,2,"ab"]]"#),
        // A body that can match text as well as hold a `\G`.
        (
            r"(?:\K\Ga?)*?b",
            r#"[[2,1,"b"]]"#,
            r#"[[2,1,"b"],[3,1,"b"]]"#,
        ),
        (r"(?i:\Ga*)*?b+", r#"[[2,1,"b"]]"#, r#"[[2,2,"bb"]]"#),
        (
            r"(?:\G(?:a|))??b",
            r#"[[2,1,"b"]]"#,
            r#"[[2,1,"b"],[3,1,"b"]]"#,
        ),
        (
            r"(?:(?=a)\G){0,}?(?=b)",
            r#"[[2,0,""]]"#,
            r#"[[2,0,""],[3,0,""]]"#,
        ),
        (r"(?:\Gb?){0,2}?$", r#"[[3,0,""]]"#, r#"[[4,0,""]]"#),
        (r"(?x: \Gc? ){0,1}?b+", r#"[[2,1,"b"]]"#, r#"[[2,2,"bb"]]"#),
        (r"(?>\Ga??)??b+", r#"[[2,1,"b"]]"#, r#"[[2,2,"bb"]]"#),
        // A `#` that is escaped or in a class opens no comment.
        (
            r"[#]?(?:\K\G)*?b",
            r#"[[2,1,"b"]]"#,
            r#"[[2,1,"b"],[3,1,"b"]]"#,
        ),
        (
            r"\#?(?:\K\G)*?b",
            r#"[[2,1,"b"]]"#,
            r#"[[2,1,"b"],[3,1,"b"]]"#,
        ),
        // The same group repeated greedily, or with a capture, answered before.
        (r"(?:\K\G)*b", r#"[[2,1,"b"]]"#, r#"[[2,1,"b"],[3,1,"b"]]"#),
        (r"(\K\G)*?b", r#"[[2,1,"b"]]"#, r#"[[2,1,"b"],[3,1,"b"]]"#),
        // A minimum of one or more never reaches the `b` from a later start, in jq as here.
        (r"(?:\K\G)+?b", "[]", "[]"),
        (r"(?:\K\G){2,3}?b", "[]", "[]"),
    ] {
        for (subject, expected) in [("cab", on_cab), ("cabb", on_cabb)] {
            let (stdout, stderr, code) = matches(subject, pattern)?;
            assert_eq!(code, 0, "{subject} {pattern}: stderr={stderr:?}");
            assert_eq!(stdout.trim_end(), expected, "{subject} {pattern}");
        }
    }
    Ok(())
}

/// A match that needs the repeat's body to run, or that has nothing after it, is as before; a lone
/// `\G` or `\K` is still refused, as jq refuses it as a repeat target.
#[test]
fn test_a_lazy_repeat_of_a_g_k_body_keeps_its_other_answers_6166() -> Result<()> {
    for (subject, pattern, expected) in [
        ("cab", r"(?:\K\G)*?(?:a|\K)b", r#"[[1,2,"ab"]]"#),
        (
            "cab",
            r"(?:\K\G)*?",
            r#"[[0,0,""],[1,0,""],[2,0,""],[3,0,""]]"#,
        ),
    ] {
        let (stdout, stderr, code) = matches(subject, pattern)?;
        assert_eq!(code, 0, "{pattern}: stderr={stderr:?}");
        assert_eq!(stdout.trim_end(), expected, "{subject} {pattern}");
    }
    for pattern in [
        r"(?:\G)*?b",
        r"(?:\K)*?b",
        r"(?:(?:\G))*?b",
        r"(?:\K|\G)*?b",
    ] {
        let (stdout, stderr, code) = matches("cab", pattern)?;
        assert_eq!(code, 5, "{pattern}: stdout={stdout:?} stderr={stderr:?}");
        assert!(stderr.contains("Regex failure"), "{pattern}: {stderr:?}");
    }
    Ok(())
}
