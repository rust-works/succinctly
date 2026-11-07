//! #6492: a capture group that holds only a zero-width anchor, after a nullable item in the same way
//! (`(\w*)(\z)`), is set by the copy of the way that consumes text too. The group-copies rewrite wrote
//! that copy from the anchor's own text, so `(\w*)(\z)` was `(\w+)\z` there and the group read `null`
//! where jq reads `""`: `"abc" | match("(?:(a)|(\w*)(\z)){2,3}?";"g")` gave `["a","bc",null]` for the
//! first match, and jq 1.7.1 answers `["a","bc",""]`. Every row was captured from jq 1.7.1; the grid
//! behind them (7 nullable items, 9 anchor spellings, 3 other ways, 10 quantifiers, 7 strings: 1,890
//! patterns) fixed 130 patterns and changed none the other way.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_group_around_an_anchor_is_set_by_the_way_that_consumes_text_6492() -> Result<()> {
    for (pattern, input, expected) in [
        (
            "(?:(a)|(\\w*)(\\z)){2}",
            "abc",
            "[[0,\"abc\",[\"a\",\"bc\",\"\"]],[3,\"\",[\"\",\"\",\"\"]]]",
        ),
        (
            "(?:(a)|(\\w*)(\\b)){2}",
            "abc",
            "[[0,\"abc\",[\"a\",\"bc\",\"\"]],[3,\"\",[\"\",\"\",\"\"]]]",
        ),
        (
            "(?:(a)|(\\w*)(\\B))*",
            "cba",
            "[[0,\"cba\",[\"a\",\"cb\",\"\"]],[3,\"\",[\"\",\"\",\"\"]]]",
        ),
        (
            "(?:(a)|(\\w*)(\\z)){0,3}",
            "aab",
            "[[0,\"aab\",[\"a\",\"b\",\"\"]],[3,\"\",[\"\",\"\",\"\"]]]",
        ),
        (
            "(?:(a)|(\\w*)(\\z)){2,3}",
            "aab",
            "[[0,\"aab\",[\"a\",\"b\",\"\"]],[3,\"\",[\"\",\"\",\"\"]]]",
        ),
        (
            "(?:(a)|(\\w*)(\\z)){2,3}?",
            "abc",
            "[[0,\"abc\",[\"a\",\"bc\",\"\"]],[3,\"\",[\"\",\"\",\"\"]]]",
        ),
        (
            "(?:(a|b)|(\\w*)(\\z)){0,3}",
            "abc",
            "[[0,\"abc\",[\"b\",\"c\",\"\"]],[3,\"\",[\"\",\"\",\"\"]]]",
        ),
        (
            "(?:(a|b)|(\\w*)(\\z)){2,3}",
            "abc",
            "[[0,\"abc\",[\"b\",\"c\",\"\"]],[3,\"\",[\"\",\"\",\"\"]]]",
        ),
        (
            "(?:(a)|(\\w*)(\\b)){0,3}",
            "aab",
            "[[0,\"aab\",[\"a\",\"b\",\"\"]],[3,\"\",[\"\",\"\",\"\"]]]",
        ),
        (
            "(?:(a)|(\\w*)(\\b)){2,3}",
            "aab",
            "[[0,\"aab\",[\"a\",\"b\",\"\"]],[3,\"\",[\"\",\"\",\"\"]]]",
        ),
        (
            "(?:(a)|(\\w*)(\\b)){2,3}?",
            "abc",
            "[[0,\"abc\",[\"a\",\"bc\",\"\"]],[3,\"\",[\"\",\"\",\"\"]]]",
        ),
        (
            "(?:(a|b)|(\\w*)(\\b)){0,3}",
            "abc",
            "[[0,\"abc\",[\"b\",\"c\",\"\"]],[3,\"\",[\"\",\"\",\"\"]]]",
        ),
        (
            "(?:(a|b)|(\\w*)(\\b)){2,3}",
            "abc",
            "[[0,\"abc\",[\"b\",\"c\",\"\"]],[3,\"\",[\"\",\"\",\"\"]]]",
        ),
        (
            "(?:(a)|(\\w*)(\\B))+",
            "cba",
            "[[0,\"cba\",[\"a\",\"cb\",\"\"]]]",
        ),
        (
            "(?:(a)|(\\w*)(\\B)){2}",
            "cba",
            "[[0,\"cba\",[\"a\",\"cb\",\"\"]]]",
        ),
        (
            "(?:(a)|(\\w*)(\\B)){2,}",
            "cba",
            "[[0,\"cba\",[\"a\",\"cb\",\"\"]]]",
        ),
        (
            "(?:(a)|(\\w*)(\\B)){0,3}",
            "cba",
            "[[0,\"cba\",[\"a\",\"cb\",\"\"]],[3,\"\",[\"\",\"\",\"\"]]]",
        ),
        (
            "(?:(a)|(\\w*)(\\B)){2,3}",
            "cba",
            "[[0,\"cba\",[\"a\",\"cb\",\"\"]]]",
        ),
        (
            "(?:(a)|(\\w*)(\\B)){2,3}?",
            "cba",
            "[[0,\"cba\",[\"a\",\"cb\",\"\"]]]",
        ),
        (
            "(?:(a|b)|(\\w*)(\\B))*",
            "cba",
            "[[0,\"cba\",[\"a\",\"cb\",\"\"]],[3,\"\",[\"\",\"\",\"\"]]]",
        ),
        (
            "(?:(a|b)|(\\w*)(\\B))+",
            "cba",
            "[[0,\"cba\",[\"a\",\"cb\",\"\"]]]",
        ),
        (
            "(?:(a|b)|(\\w*)(\\B)){2}",
            "cba",
            "[[0,\"cba\",[\"a\",\"cb\",\"\"]]]",
        ),
        (
            "(?:(a|b)|(\\w*)(\\B)){2,}",
            "cba",
            "[[0,\"cba\",[\"a\",\"cb\",\"\"]]]",
        ),
        (
            "(?:(a|b)|(\\w*)(\\B)){0,3}",
            "cba",
            "[[0,\"cba\",[\"a\",\"cb\",\"\"]],[3,\"\",[\"\",\"\",\"\"]]]",
        ),
        (
            "(?:(a|b)|(\\w*)(\\B)){2,3}",
            "cba",
            "[[0,\"cba\",[\"a\",\"cb\",\"\"]]]",
        ),
        (
            "(?:(a|b)|(\\w*)(\\B)){2,3}?",
            "cba",
            "[[0,\"cba\",[\"a\",\"cb\",\"\"]]]",
        ),
    ] {
        let (stdout, _, status) = run_jq_full(
            &[
                "-nc",
                "--arg",
                "s",
                input,
                "--arg",
                "p",
                pattern,
                r#"[$s | match($p; "g") | [.offset, .string, (.captures | map(.string))]]"#,
            ],
            None,
        )?;
        assert_eq!(
            (stdout.trim_end(), status),
            (expected, 0),
            "{pattern} on {input:?}"
        );
    }
    Ok(())
}
