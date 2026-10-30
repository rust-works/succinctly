//! #5572: jq's `$` matches at the end of the text or just before a final `\n`. `succinctly jq`
//! searches the text and the text without that `\n` and picks one match; the pick was wrong when
//! both views matched at the same start (a greedy `\s+$` takes the `\n`, a lazy one does not) and
//! the second view was skipped when the text ended in two newlines. Every row was captured from
//! jq 1.7.1 (`/usr/bin/jq -nc`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_dollar_before_a_final_newline_matches_jq_5572() -> Result<()> {
    for (filter, expected) in [
        (r#""abc \n" | sub("\\s+$"; "")"#, r#""abc""#),
        (
            r#""abc \n" | [match("\\s+$") | [.offset, .length]]"#,
            "[[3,2]]",
        ),
        (r#""abc \n" | [scan("\\s+$")]"#, r#"[" \n"]"#),
        (r#""abc \n" | [splits("\\s+$")]"#, r#"["abc",""]"#),
        (r#""abc \n" | sub("\\s+?$"; "")"#, r#""abc\n""#),
        (
            r#""abc \n" | [match("\\s+?$") | [.offset, .length]]"#,
            "[[3,1]]",
        ),
        (r#""abc \n" | gsub("\\s+$"; "")"#, r#""abc""#),
        (r#""abc " | sub("\\s+$"; "")"#, r#""abc""#),
        (
            r#""a \n \n" | [match("\\s+$"; "g") | [.offset, .length]]"#,
            "[[1,4]]",
        ),
        (
            r#""k=v \n" | capture("(?<k>\\w+)=(?<v>\\w+)\\s*$")"#,
            r#"{"k":"k","v":"v"}"#,
        ),
        (r#""a\n\n" | [match("$"; "g") | .offset]"#, "[2,3]"),
        (r#""a\n\n" | [match("\\s$"; "g") | .offset]"#, "[1,2]"),
        (r#""a\n\n" | test("a\\s$")"#, "true"),
        (r#""a\n\n" | test("a$")"#, "false"),
        (r#""a\n\n" | test("a\\n$")"#, "true"),
        (r#""line\n\n" | gsub("\\n$"; "")"#, r#""line""#),
        (r#""line\n\n" | sub("\\n$"; "")"#, r#""line\n""#),
        (r#""a\n" | sub("a$"; "X")"#, r#""X\n""#),
        (r#""a\n\n" | [splits("$")]"#, r#"["a\n","\n",""]"#),
        (r#""ab\n\n" | [scan("b\\s*$")]"#, r#"["b\n\n"]"#),
        (r#""x\n" | [match("x\\s*$|x$") | .length]"#, "[2]"),
        (r#""x \n" | [match("x\\s*$|x$") | .length]"#, "[3]"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(
            (stdout.trim_end(), code),
            (expected, 0),
            "{filter}: stderr {stderr}"
        );
    }
    Ok(())
}
