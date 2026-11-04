//! #6164: an option group whose body is a zero-width item (`\K`, `\G`, a lookaround) is a valid
//! target of a repeat in Oniguruma, where `fancy-regex` refuses it and the `regex` crate cannot read
//! the escape: `(?i:\K)*` answered `invalid regex: regex parse error`. The group is written as an
//! atomic one for the engine. Every row was captured from jq 1.7.1 (`/usr/bin/jq -n`).

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
fn test_a_repeated_option_group_of_a_zero_width_body_answers_as_jq_6164() -> Result<()> {
    for (pattern, expected) in [
        (r"(?i:\K)*", r#"[[0,0,""],[1,0,""],[2,0,""],[3,0,""]]"#),
        (r"(?i:\G)+", r#"[[0,0,""],[1,0,""],[2,0,""],[3,0,""]]"#),
        (r"(?-i:\K)*", r#"[[0,0,""],[1,0,""],[2,0,""],[3,0,""]]"#),
        (r"(?x:\K)*", r#"[[0,0,""],[1,0,""],[2,0,""],[3,0,""]]"#),
        (r"(?s:\K)*", r#"[[0,0,""],[1,0,""],[2,0,""],[3,0,""]]"#),
        (r"(?i:(?:\K))*", r#"[[0,0,""],[1,0,""],[2,0,""],[3,0,""]]"#),
        (r"(?i:(?=a))*", r#"[[0,0,""],[1,0,""],[2,0,""],[3,0,""]]"#),
        (r"(?i:(?!a))+", r#"[[0,0,""],[2,0,""],[3,0,""]]"#),
        (r"(?i:(?<=a))*", r#"[[0,0,""],[1,0,""],[2,0,""],[3,0,""]]"#),
        (r"a(?i:\K)*b", r#"[[2,1,"b"]]"#),
        (r"a(?i:\K)+b", r#"[[2,1,"b"]]"#),
        (r"a(?i:\K)*?b", r#"[[1,2,"ab"]]"#),
        (r"a(?i:\K)?b", r#"[[2,1,"b"]]"#),
        (r"a(?i:\G)*b", r#"[[1,2,"ab"]]"#),
        (r"a(?i:\G)+", r#"[]"#),
        (r"(?i:\K){2}", r#"[[0,0,""],[1,0,""],[2,0,""],[3,0,""]]"#),
        (r"(?i:\K){0,2}", r#"[[0,0,""],[1,0,""],[2,0,""],[3,0,""]]"#),
        (r"(?i:\K){,2}", r#"[]"#),
        (r"(?i:\G)*?b", r#"[[2,1,"b"]]"#),
        (r"(?i:\G)??b", r#"[[2,1,"b"]]"#),
        (r"(?i:(?=a))*?a", r#"[[1,1,"a"]]"#),
        (r"(?i:\K)(?i:\G)*", r#"[[0,0,""],[1,0,""],[2,0,""],[3,0,""]]"#),
        (r"(?i:a)*(?i:\K)*c", r#"[[0,1,"c"]]"#),
        (r"(?i:\K\G)*b", r#"[[2,1,"b"]]"#),
    ] {
        let (stdout, stderr, code) = matches(pattern)?;
        assert_eq!(code, 0, "{pattern}: stderr={stderr:?}");
        assert_eq!(stdout.trim_end(), expected, "{pattern}");
    }
    Ok(())
}

/// jq refuses a repeat of a plain `(?:...)` group or a bare escape that it reads as an assertion
/// (#6127), and so does this; only the option group is a valid target.
#[test]
fn test_a_repeated_plain_group_of_a_zero_width_body_stays_refused_6164() -> Result<()> {
    for pattern in [r"(?:\K)*", r"(?:\G)+", r"\K*", r"(?:(?=a))*", r"(?:\K)?*"] {
        let (stdout, stderr, code) = matches(pattern)?;
        assert_eq!(code, 5, "{pattern}: stdout={stdout:?} stderr={stderr:?}");
        assert!(stderr.contains("invalid regex"), "{pattern}: {stderr:?}");
    }
    Ok(())
}
