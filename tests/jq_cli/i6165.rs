//! #6165: a stacked repeat (`a**`) is valid in Oniguruma, and `fancy-regex` reads it in its
//! Oniguruma mode, but the pattern was first parsed in its default mode, which refuses it: a group
//! holding a `\K` (the `fancy-regex` route) answered `invalid regex` for `(?:\Ka)**` where
//! `(?:ab)**` was accepted by the `regex` crate. Every row was captured from jq 1.7.1
//! (`/usr/bin/jq -n`).

use anyhow::Result;

use crate::run_jq_full;

/// `"cab" | [match(PATTERN; "g") | [.offset, .length, .string]]`, compact.
fn matches(pattern: &str) -> Result<(String, String, i32)> {
    let filter = format!(
        r#""cab" | [match("{}"; "g") | [.offset, .length, .string]]"#,
        pattern.replace('\\', "\\\\").replace('"', "\\\"")
    );
    run_jq_full(&["-nc", &filter], None)
}

#[test]
fn test_a_stacked_repeat_of_a_group_holding_a_keep_answers_as_jq_6165() -> Result<()> {
    for (pattern, expected) in [
        (r"(?:\Ka)**", r#"[[0,0,""],[1,1,"a"],[2,0,""],[3,0,""]]"#),
        (r"(?:\Ka)+*", r#"[[0,0,""],[1,1,"a"],[2,0,""],[3,0,""]]"#),
        (r"(?:\Ka){2}*", r#"[[0,0,""],[1,0,""],[2,0,""],[3,0,""]]"#),
        (r"(?:\Ka)*?*", r#"[[0,0,""],[1,0,""],[2,0,""],[3,0,""]]"#),
        (r"(?:ab)**", r#"[[0,0,""],[1,2,"ab"],[3,0,""]]"#),
        (r"a(?:\Ka)**b", r#"[[1,2,"ab"]]"#),
        (r"(\Ka)**", r#"[[0,0,""],[1,1,"a"],[2,0,""],[3,0,""]]"#),
        (r"(?:a\K)??*", r#"[[0,0,""],[1,0,""],[2,0,""],[3,0,""]]"#),
        (r"(?:\Ka)*{2}", r#"[[0,0,""],[1,1,"a"],[2,0,""],[3,0,""]]"#),
    ] {
        let (stdout, stderr, code) = matches(pattern)?;
        assert_eq!(code, 0, "{pattern}: stderr={stderr:?}");
        assert_eq!(stdout.trim_end(), expected, "{pattern}");
    }
    Ok(())
}

/// Oniguruma's mode reads a repeat that follows only a comment as a repeat of an empty group, where
/// jq refuses it, because a comment is invisible to jq (`a(?#)?` is `a?`, a bare `(?#)?` is a repeat
/// with no target). The pattern is read again without its comments to keep the refusal.
#[test]
fn test_a_repeat_with_only_a_comment_before_it_stays_refused_6165() -> Result<()> {
    for pattern in [r"(?#)?", "(?#c)+", "(?:(?#)?)", "a|(?#x)*", r"(?#a\)b)?"] {
        let (stdout, stderr, code) = matches(pattern)?;
        assert_eq!(code, 5, "{pattern}: stdout={stdout:?} stderr={stderr:?}");
        assert!(stderr.contains("Regex failure"), "{pattern}: {stderr:?}");
    }
    for (pattern, expected) in [
        ("a(?#)?", r#"[[0,0,""],[1,1,"a"],[2,0,""],[3,0,""]]"#),
        (
            r"(?:\Ka)(?#c)**",
            r#"[[0,0,""],[1,1,"a"],[2,0,""],[3,0,""]]"#,
        ),
    ] {
        let (stdout, stderr, code) = matches(pattern)?;
        assert_eq!(code, 0, "{pattern}: stderr={stderr:?}");
        assert_eq!(stdout.trim_end(), expected, "{pattern}");
    }
    Ok(())
}
