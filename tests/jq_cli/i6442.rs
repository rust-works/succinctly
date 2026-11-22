//! #6442: a loop with capture groups answers as Oniguruma's does under jq's `x` flag and a leading `(?x)`.
//! The group-copy rewrite stood down for a switch that names `x`, so the loop kept the `regex` crate's last
//! iteration: `"bab" | [match("(()(?:a|)|(b*))*";"gx") | .string]` was `["","ab",""]` where jq 1.7.1 answers
//! `["","a","",""]`. Every row was captured from jq 1.7.1 (`jq -nc`).
//! Pin moved (#7520): a row whose empty overall match leaves a capture group unset is jq 1.8.2's answer
//! (`null`, offset -1), where jq 1.7.1 reported an empty string.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_loop_with_groups_under_the_x_flag_answers_as_jq_does_6442() -> Result<()> {
    for (pattern, flags, input, expected) in [
        ("(()(?:a|)|(b*))*", "gx", "bab", "[[0,\"\",[\"\",\"\",null]],[1,\"a\",[\"\",\"\",null]],[2,\"\",[\"\",\"\",null]],[3,\"\",[\"\",\"\",null]]]"),
        ("(()(?:a|)|(b*))*", "gx", "abc", "[[0,\"a\",[\"\",\"\",null]],[1,\"\",[\"\",\"\",null]],[2,\"\",[\"\",\"\",null]],[3,\"\",[\"\",\"\",null]]]"),
        ("(()(?:a|)|(b*))*", "gx", "aab", "[[0,\"aa\",[\"\",\"\",null]],[2,\"\",[\"\",\"\",null]],[3,\"\",[\"\",\"\",null]]]"),
        ("(()(?:a|)|(b*))*", "gx", "", "[[0,\"\",[\"\",\"\",null]]]"),
        ("(()|(b*)){1,2}c", "gx", "abc", "[[1,\"bc\",[\"\",\"\",\"b\"]]]"),
        ("(()|(b*)){1,2}c", "gx", "bc", "[[0,\"bc\",[\"\",\"\",\"b\"]]]"),
        ("(()|(b*)){1,2}c", "gx", "c", "[[0,\"c\",[\"\",\"\",null]]]"),
        ("(?: (a) | b* )+ \\ ", "gx", "ba ", "[[0,\"ba \",[\"a\"]]]"),
        ("(?: (a) | b* )+ \\ ", "gx", "aab ", "[[0,\"aab \",[\"a\"]]]"),
        ("(?: (a) | b* )+ \\ ", "gx", "b ", "[[0,\"b \",[null]]]"),
        ("( (?: b* | (a) )+ ) (b)", "gx", "bab", "[[0,\"bab\",[\"ba\",\"a\",\"b\"]]]"),
        ("( (?: b* | (a) )+ ) (b)", "gx", "aab", "[[0,\"aab\",[\"aa\",\"a\",\"b\"]]]"),
        ("( (?: b* | (a) )+ ) (b)", "gx", "ab", "[[0,\"ab\",[\"a\",\"a\",\"b\"]]]"),
        ("(?x)(?:(a)|b*)+", "g", "ba", "[[0,\"ba\",[\"a\"]],[2,\"\",[null]]]"),
        ("(?x)(?:(a)|b*)+", "g", "aab", "[[0,\"aab\",[\"a\"]],[3,\"\",[null]]]"),
        ("(?x)(?:(a)|b*)+", "g", "bab", "[[0,\"bab\",[\"a\"]],[3,\"\",[null]]]"),
        ("(?x) (?: (a) | () ){2,}", "g", "ba", "[[0,\"\",[null,\"\"]],[1,\"a\",[\"a\",\"\"]],[2,\"\",[null,\"\"]]]"),
        ("(?x) (?: (a) | () ){2,}", "g", "aab", "[[0,\"aa\",[\"a\",\"\"]],[2,\"\",[null,\"\"]],[3,\"\",[null,\"\"]]]"),
        ("(?ix)(?:(a)|B*)+", "g", "bAB", "[[0,\"bAB\",[\"A\"]],[3,\"\",[null]]]"),
        ("(?ix)(?:(a)|B*)+", "g", "aab", "[[0,\"aab\",[\"a\"]],[3,\"\",[null]]]"),
        ("(?:(a)|b*)+", "gx", "bab", "[[0,\"bab\",[\"a\"]],[3,\"\",[null]]]"),
        ("(?:(a)|b*)+", "gx", "aa", "[[0,\"aa\",[\"a\"]],[2,\"\",[null]]]"),
        ("(?i)(?x)(?: (a) | b* )+ (c)?", "g", "bAc", "[[0,\"bAc\",[\"A\",\"c\"]],[3,\"\",[null,null]]]"),
        ("(?i)(?x)(?: (a) | b* )+ (c)?", "g", "aab", "[[0,\"aab\",[\"a\",null]],[3,\"\",[null,null]]]"),
        ("(?i-x)(?:(a)|b*)+", "g", "bAB", "[[0,\"bAB\",[\"A\"]],[3,\"\",[null]]]"),
        ("(?i-x)(?:(a)|b*)+", "g", "aab", "[[0,\"aab\",[\"a\"]],[3,\"\",[null]]]"),
        ("(?-x)(?:(a)|b*)+ ", "g", "ba ", "[[0,\"ba \",[\"a\"]]]"),
        ("(?-x)(?:(a)|b*)+ ", "g", "aab", "[]"),
        ("(?x)(?:(a)|b*)+", "gi", "bAB", "[[0,\"bAB\",[\"A\"]],[3,\"\",[null]]]"),
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
                "--arg",
                "f",
                flags,
                "[$s | match($p; $f) | [.offset, .string, (.captures | map(.string))]]",
            ],
            None,
        )?;
        assert_eq!(
            (stdout.trim_end(), status),
            (expected, 0),
            "{pattern:?} ({flags}) on {input:?}"
        );
    }
    Ok(())
}
