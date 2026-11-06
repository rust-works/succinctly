//! #6433: a count past Oniguruma's limit (100000) is refused with `too big number for repeat range` wherever
//! jq reads it: after a `{` whatever follows (`a{100001 }`, `a{100001x}`), with nothing to repeat (`{100001}`),
//! and inside a look-around or an atomic group, which the `regex` crate's parser does not read. A `{` that
//! starts no count (`a{ 100001}`, `a{1 ,100001}`, `a{,100001}`), an escape, a class, a `(?#..)` comment, a
//! `\Q..\E` run and an extended-mode comment are not counts. Every row was captured from jq 1.7.1 (`jq -nc`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_count_past_oniguruma_s_limit_is_refused_where_jq_reads_it_6433() -> Result<()> {
    for (pattern, flags, expected) in [
        (
            "a{100001 }",
            "",
            "\"Regex failure: too big number for repeat range\"",
        ),
        (
            "a{100001, }",
            "",
            "\"Regex failure: too big number for repeat range\"",
        ),
        (
            "a{1,100001 }",
            "",
            "\"Regex failure: too big number for repeat range\"",
        ),
        (
            "a{100001x}",
            "",
            "\"Regex failure: too big number for repeat range\"",
        ),
        (
            "a{100001",
            "",
            "\"Regex failure: too big number for repeat range\"",
        ),
        (
            "a{1,100001x}",
            "",
            "\"Regex failure: too big number for repeat range\"",
        ),
        (
            "a{100001 , 5}",
            "",
            "\"Regex failure: too big number for repeat range\"",
        ),
        (
            "{100001}",
            "",
            "\"Regex failure: too big number for repeat range\"",
        ),
        (
            "{100001,}",
            "",
            "\"Regex failure: too big number for repeat range\"",
        ),
        (
            "{1,100001}",
            "",
            "\"Regex failure: too big number for repeat range\"",
        ),
        (
            "(?:a|{100001})",
            "",
            "\"Regex failure: too big number for repeat range\"",
        ),
        (
            "(?=a{100001})",
            "",
            "\"Regex failure: too big number for repeat range\"",
        ),
        (
            "(?!a{100001})",
            "",
            "\"Regex failure: too big number for repeat range\"",
        ),
        (
            "(?<=a{100001})",
            "",
            "\"Regex failure: too big number for repeat range\"",
        ),
        (
            "(?>a{100001})",
            "",
            "\"Regex failure: too big number for repeat range\"",
        ),
        (
            "(?=a{1,100001})",
            "",
            "\"Regex failure: too big number for repeat range\"",
        ),
        (
            "(?=a{99999999999})",
            "",
            "\"Regex failure: too big number for repeat range\"",
        ),
        (
            "a{000100001}",
            "",
            "\"Regex failure: too big number for repeat range\"",
        ),
        (
            "\\Qa\\E{100001}",
            "",
            "\"Regex failure: too big number for repeat range\"",
        ),
        (
            "a{100001 }",
            "x",
            "\"Regex failure: too big number for repeat range\"",
        ),
        (
            "a {100001 }",
            "x",
            "\"Regex failure: too big number for repeat range\"",
        ),
        (
            "(?=a{100001})",
            "x",
            "\"Regex failure: too big number for repeat range\"",
        ),
        (
            "a{100001}",
            "g",
            "\"Regex failure: too big number for repeat range\"",
        ),
        ("a{ 100001}", "", "false"),
        ("a{1 ,100001}", "", "false"),
        ("a{1, 100001}", "", "false"),
        ("a{,100001}", "", "false"),
        ("a{100000}", "", "false"),
        ("a{0100000}", "", "false"),
        ("\\{100001}", "", "false"),
        ("\\x{10FFFF}a", "", "false"),
        ("[{100001}]", "", "false"),
        ("(?#a{100001})a", "", "true"),
        ("\\Qa{100001}\\E", "", "false"),
        ("a # {100001}", "x", "true"),
        ("(?x)a # {100001}", "", "true"),
        ("a{100001}", "z", "\"z is not a valid modifier string\""),
        ("\\x{100001}a", "", "false"),
        ("\\x{100001}{2}", "", "false"),
    ] {
        let (stdout, _, status) = run_jq_full(
            &[
                "-nc",
                "--arg",
                "p",
                pattern,
                "--arg",
                "f",
                flags,
                r#""a" | try test($p; $f) catch ."#,
            ],
            None,
        )?;
        assert_eq!(
            (stdout.trim_end(), status),
            (expected, 0),
            "{pattern:?} ({flags:?})"
        );
    }
    Ok(())
}
