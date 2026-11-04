//! #6215: under the `x` flag a quantifier on an assertion, or on a group that reads as one
//! (`\b*`, `^+`, `(?:\B){1,2}`, `(?:a|\b)*`), was accepted where jq refuses it with `target of repeat
//! operator is invalid`, because the blanks and comments the flag ignores hid what the quantifier
//! follows. The checks now read the pattern without them. Every row was captured from jq 1.7.1
//! (`/usr/bin/jq -n`); an error row pins that the pattern is refused here too.

use anyhow::Result;

use crate::run_jq_full;

/// (flags, subject, whether jq refuses the pattern, jq's stdout when it does not).
type Answer = (&'static str, &'static str, bool, &'static str);

/// `$s | [match($p; $f) | [.offset, .length, .string]]`, compact: whether it failed, and stdout.
fn matches(subject: &str, pattern: &str, flags: &str) -> Result<(bool, String)> {
    let (stdout, _stderr, code) = run_jq_full(
        &[
            "-nc",
            "--arg",
            "p",
            pattern,
            "--arg",
            "f",
            flags,
            "--arg",
            "s",
            subject,
            r"$s | [match($p; $f) | [.offset, .length, .string]]",
        ],
        None,
    )?;
    Ok((code != 0, stdout.trim_end().to_string()))
}

/// (pattern, the answer for each flag and subject).
const ROWS: &[(&str, &[Answer])] = &[
    (
        "\\b*",
        &[
            ("gx", "cab", true, ""),
            ("gx", "a b", true, ""),
            ("x", "cab", true, ""),
            ("x", "a b", true, ""),
        ],
    ),
    (
        "^+",
        &[
            ("gx", "cab", true, ""),
            ("gx", "a b", true, ""),
            ("x", "cab", true, ""),
            ("x", "a b", true, ""),
        ],
    ),
    (
        "a$?",
        &[
            ("gx", "cab", true, ""),
            ("gx", "a b", true, ""),
            ("x", "cab", true, ""),
            ("x", "a b", true, ""),
        ],
    ),
    (
        "(?:\\B){1,2}",
        &[
            ("gx", "cab", true, ""),
            ("gx", "a b", true, ""),
            ("x", "cab", true, ""),
            ("x", "a b", true, ""),
        ],
    ),
    (
        "(?:a|\\b)*",
        &[
            ("gx", "cab", true, ""),
            ("gx", "a b", true, ""),
            ("x", "cab", true, ""),
            ("x", "a b", true, ""),
        ],
    ),
    (
        "(?i)*a",
        &[
            ("gx", "cab", true, ""),
            ("gx", "a b", true, ""),
            ("x", "cab", true, ""),
            ("x", "a b", true, ""),
        ],
    ),
    (
        "^ *",
        &[
            ("gx", "cab", true, ""),
            ("gx", "a b", true, ""),
            ("x", "cab", true, ""),
            ("x", "a b", true, ""),
        ],
    ),
    (
        "\\b +",
        &[
            ("gx", "cab", true, ""),
            ("gx", "a b", true, ""),
            ("x", "cab", true, ""),
            ("x", "a b", true, ""),
        ],
    ),
    (
        "(?: \\b )*",
        &[
            ("gx", "cab", true, ""),
            ("gx", "a b", true, ""),
            ("x", "cab", true, ""),
            ("x", "a b", true, ""),
        ],
    ),
    (
        "a *",
        &[
            (
                "gx",
                "cab",
                false,
                r#"[[0,0,""],[1,1,"a"],[2,0,""],[3,0,""]]"#,
            ),
            (
                "gx",
                "a b",
                false,
                r#"[[0,1,"a"],[1,0,""],[2,0,""],[3,0,""]]"#,
            ),
            ("x", "cab", false, r#"[[0,0,""]]"#),
            ("x", "a b", false, r#"[[0,1,"a"]]"#),
        ],
    ),
    (
        "a {2}",
        &[
            ("gx", "cab", false, r"[]"),
            ("gx", "a b", false, r"[]"),
            ("x", "cab", false, r"[]"),
            ("x", "a b", false, r"[]"),
        ],
    ),
    (
        "a\\ *",
        &[
            ("gx", "cab", false, r#"[[1,1,"a"]]"#),
            ("gx", "a b", false, r#"[[0,2,"a "]]"#),
            ("x", "cab", false, r#"[[1,1,"a"]]"#),
            ("x", "a b", false, r#"[[0,2,"a "]]"#),
        ],
    ),
    (
        "[ ]*",
        &[
            (
                "gx",
                "cab",
                false,
                r#"[[0,0,""],[1,0,""],[2,0,""],[3,0,""]]"#,
            ),
            (
                "gx",
                "a b",
                false,
                r#"[[0,0,""],[1,1," "],[2,0,""],[3,0,""]]"#,
            ),
            ("x", "cab", false, r#"[[0,0,""]]"#),
            ("x", "a b", false, r#"[[0,0,""]]"#),
        ],
    ),
    (
        "[#]*",
        &[
            (
                "gx",
                "cab",
                false,
                r#"[[0,0,""],[1,0,""],[2,0,""],[3,0,""]]"#,
            ),
            (
                "gx",
                "a b",
                false,
                r#"[[0,0,""],[1,0,""],[2,0,""],[3,0,""]]"#,
            ),
            ("x", "cab", false, r#"[[0,0,""]]"#),
            ("x", "a b", false, r#"[[0,0,""]]"#),
        ],
    ),
    (
        "(?:\\b | a)*",
        &[
            ("gx", "cab", true, ""),
            ("gx", "a b", true, ""),
            ("x", "cab", true, ""),
            ("x", "a b", true, ""),
        ],
    ),
    (
        "(?: a | \\b b)*",
        &[
            (
                "gx",
                "cab",
                false,
                r#"[[0,0,""],[1,1,"a"],[2,0,""],[3,0,""]]"#,
            ),
            (
                "gx",
                "a b",
                false,
                r#"[[0,1,"a"],[1,0,""],[2,1,"b"],[3,0,""]]"#,
            ),
            ("x", "cab", false, r#"[[0,0,""]]"#),
            ("x", "a b", false, r#"[[0,1,"a"]]"#),
        ],
    ),
    (
        "\\b # note\n*",
        &[
            ("gx", "cab", true, ""),
            ("gx", "a b", true, ""),
            ("x", "cab", true, ""),
            ("x", "a b", true, ""),
        ],
    ),
    (
        "(a )*",
        &[
            (
                "gx",
                "cab",
                false,
                r#"[[0,0,""],[1,1,"a"],[2,0,""],[3,0,""]]"#,
            ),
            (
                "gx",
                "a b",
                false,
                r#"[[0,1,"a"],[1,0,""],[2,0,""],[3,0,""]]"#,
            ),
            ("x", "cab", false, r#"[[0,0,""]]"#),
            ("x", "a b", false, r#"[[0,1,"a"]]"#),
        ],
    ),
    (
        "^ ",
        &[
            ("gx", "cab", false, r#"[[0,0,""]]"#),
            ("gx", "a b", false, r#"[[0,0,""]]"#),
            ("x", "cab", false, r#"[[0,0,""]]"#),
            ("x", "a b", false, r#"[[0,0,""]]"#),
        ],
    ),
    (
        "\\b\\b *",
        &[
            ("gx", "cab", true, ""),
            ("gx", "a b", true, ""),
            ("x", "cab", true, ""),
            ("x", "a b", true, ""),
        ],
    ),
    (
        "(?:^ )?",
        &[
            ("gx", "cab", true, ""),
            ("gx", "a b", true, ""),
            ("x", "cab", true, ""),
            ("x", "a b", true, ""),
        ],
    ),
    (
        "(\\b)*",
        &[
            (
                "gx",
                "cab",
                false,
                r#"[[0,0,""],[1,0,""],[2,0,""],[3,0,""]]"#,
            ),
            (
                "gx",
                "a b",
                false,
                r#"[[0,0,""],[1,0,""],[2,0,""],[3,0,""]]"#,
            ),
            ("x", "cab", false, r#"[[0,0,""]]"#),
            ("x", "a b", false, r#"[[0,0,""]]"#),
        ],
    ),
    (
        "a ^ *",
        &[
            ("gx", "cab", true, ""),
            ("gx", "a b", true, ""),
            ("x", "cab", true, ""),
            ("x", "a b", true, ""),
        ],
    ),
    (
        "\\b{1, 2}",
        &[
            ("gx", "cab", false, r"[]"),
            ("gx", "a b", false, r"[]"),
            ("x", "cab", false, r"[]"),
            ("x", "a b", false, r"[]"),
        ],
    ),
    (
        "^{1, 2}a",
        &[
            ("gx", "cab", false, r"[]"),
            ("gx", "a b", false, r"[]"),
            ("x", "cab", false, r"[]"),
            ("x", "a b", false, r"[]"),
        ],
    ),
    (
        "(a)\\1 0",
        &[
            ("gx", "cab", false, r"[]"),
            ("gx", "a b", false, r"[]"),
            ("x", "cab", false, r"[]"),
            ("x", "a b", false, r"[]"),
        ],
    ),
    (
        "(a)\\1 *",
        &[
            ("gx", "cab", false, r#"[[1,1,"a"]]"#),
            ("gx", "a b", false, r#"[[0,1,"a"]]"#),
            ("x", "cab", false, r#"[[1,1,"a"]]"#),
            ("x", "a b", false, r#"[[0,1,"a"]]"#),
        ],
    ),
    (
        "a(?#x\\) \\b)b",
        &[
            ("gx", "cab", false, r#"[[1,2,"ab"]]"#),
            ("gx", "a b", false, r"[]"),
            ("x", "cab", false, r#"[[1,2,"ab"]]"#),
            ("x", "a b", false, r"[]"),
        ],
    ),
    (
        "a(?#x\\) )*b",
        &[
            ("gx", "cab", false, r#"[[1,2,"ab"]]"#),
            ("gx", "a b", false, r#"[[2,1,"b"]]"#),
            ("x", "cab", false, r#"[[1,2,"ab"]]"#),
            ("x", "a b", false, r#"[[2,1,"b"]]"#),
        ],
    ),
    (
        "\\b  *",
        &[
            ("gx", "cab", true, ""),
            ("gx", "a b", true, ""),
            ("x", "cab", true, ""),
            ("x", "a b", true, ""),
        ],
    ),
    (
        "\\Q \\E *",
        &[
            (
                "gx",
                "cab",
                false,
                r#"[[0,0,""],[1,0,""],[2,0,""],[3,0,""]]"#,
            ),
            (
                "gx",
                "a b",
                false,
                r#"[[0,0,""],[1,1," "],[2,0,""],[3,0,""]]"#,
            ),
            ("x", "cab", false, r#"[[0,0,""]]"#),
            ("x", "a b", false, r#"[[0,0,""]]"#),
        ],
    ),
    (
        "[\\b] *",
        &[
            (
                "gx",
                "cab",
                false,
                r#"[[0,0,""],[1,0,""],[2,0,""],[3,0,""]]"#,
            ),
            (
                "gx",
                "a b",
                false,
                r#"[[0,0,""],[1,0,""],[2,0,""],[3,0,""]]"#,
            ),
            ("x", "cab", false, r#"[[0,0,""]]"#),
            ("x", "a b", false, r#"[[0,0,""]]"#),
        ],
    ),
    (
        "a{1, 2} *",
        &[
            ("gx", "cab", false, r"[]"),
            ("gx", "a b", false, r"[]"),
            ("x", "cab", false, r"[]"),
            ("x", "a b", false, r"[]"),
        ],
    ),
];

#[test]
fn test_the_x_flag_does_not_hide_a_quantifier_on_an_assertion_6215() -> Result<()> {
    for &(pattern, expected) in ROWS {
        for &(flags, subject, refused, stdout) in expected {
            let (failed, got) = matches(subject, pattern, flags)?;
            assert_eq!(failed, refused, "{flags} {subject} {pattern:?}");
            if !refused {
                assert_eq!(got, stdout, "{flags} {subject} {pattern:?}");
            }
        }
    }
    Ok(())
}
