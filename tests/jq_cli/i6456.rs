//! #6456: under `(?m)`, Oniguruma's `^` holds at the start of the text and after a newline, but not at the
//! end of the text (`ON_STR_END`), so `"a\n" | [match("(?m)^";"g").offset]` is `[0]`. The `regex` crate's holds
//! after any newline, one more match at the end of a text that ends with one. A match of the crate that ends at
//! the end of such a text is asked of `fancy-regex`, with each `(?m)` `^` written as `(?:\A|^(?!\z))`. Every
//! row was captured from jq 1.7.1 (`jq -nc`).

use anyhow::Result;

use crate::run_jq_full;

fn run(program: &str, subject: &str, pattern: &str, flags: &str) -> Result<(String, i32)> {
    let (stdout, _, status) = run_jq_full(
        &[
            "-nc", "--arg", "s", subject, "--arg", "p", pattern, "--arg", "f", flags, program,
        ],
        None,
    )?;
    Ok((stdout.trim_end().to_owned(), status))
}

#[test]
fn a_multiline_caret_does_not_hold_at_the_end_of_the_text_6456() -> Result<()> {
    for (subject, pattern, flags, expected) in [
        ("a\n", "(?m)^", "g", "[[0,\"\",[]]]"),
        ("\n", "(?m)^", "g", "[[0,\"\",[]]]"),
        ("a\nb\n", "(?m)^", "g", "[[0,\"\",[]],[2,\"\",[]]]"),
        ("a\nb", "(?m)^", "g", "[[0,\"\",[]],[2,\"\",[]]]"),
        (
            "a\n\nb\n",
            "(?m)^",
            "g",
            "[[0,\"\",[]],[2,\"\",[]],[3,\"\",[]]]",
        ),
        ("a\n", "(?m)\\n^", "g", "[]"),
        ("\n", "(?m)\\n^", "g", "[]"),
        ("a\nb\n", "(?m)\\n^", "g", "[[1,\"\\n\",[]]]"),
        ("a\nb", "(?m)\\n^", "g", "[[1,\"\\n\",[]]]"),
        (
            "a\n\nb\n",
            "(?m)\\n^",
            "g",
            "[[1,\"\\n\",[]],[2,\"\\n\",[]]]",
        ),
        ("a\n", "(?m)^b", "g", "[]"),
        ("\n", "(?m)^b", "g", "[]"),
        ("a\nb\n", "(?m)^b", "g", "[[2,\"b\",[]]]"),
        ("a\nb", "(?m)^b", "g", "[[2,\"b\",[]]]"),
        ("a\n\nb\n", "(?m)^b", "g", "[[3,\"b\",[]]]"),
        ("a\n", "(?m)^a?", "g", "[[0,\"a\",[]]]"),
        ("\n", "(?m)^a?", "g", "[[0,\"\",[]]]"),
        ("a\nb\n", "(?m)^a?", "g", "[[0,\"a\",[]],[2,\"\",[]]]"),
        ("a\nb", "(?m)^a?", "g", "[[0,\"a\",[]],[2,\"\",[]]]"),
        (
            "a\n\nb\n",
            "(?m)^a?",
            "g",
            "[[0,\"a\",[]],[2,\"\",[]],[3,\"\",[]]]",
        ),
        ("a\n", "(?m)^\\n?", "g", "[[0,\"\",[]]]"),
        ("\n", "(?m)^\\n?", "g", "[[0,\"\\n\",[]]]"),
        ("a\nb\n", "(?m)^\\n?", "g", "[[0,\"\",[]],[2,\"\",[]]]"),
        ("a\nb", "(?m)^\\n?", "g", "[[0,\"\",[]],[2,\"\",[]]]"),
        (
            "a\n\nb\n",
            "(?m)^\\n?",
            "g",
            "[[0,\"\",[]],[2,\"\\n\",[]],[3,\"\",[]]]",
        ),
        ("a\n", "(?m)^$", "g", "[]"),
        ("\n", "(?m)^$", "g", "[[0,\"\",[]]]"),
        ("a\nb\n", "(?m)^$", "g", "[]"),
        ("a\nb", "(?m)^$", "g", "[]"),
        ("a\n\nb\n", "(?m)^$", "g", "[[2,\"\",[]]]"),
        ("a\n", "(?m)(?:^|b)", "g", "[[0,\"\",[]]]"),
        ("\n", "(?m)(?:^|b)", "g", "[[0,\"\",[]]]"),
        ("a\nb\n", "(?m)(?:^|b)", "g", "[[0,\"\",[]],[2,\"\",[]]]"),
        ("a\nb", "(?m)(?:^|b)", "g", "[[0,\"\",[]],[2,\"\",[]]]"),
        (
            "a\n\nb\n",
            "(?m)(?:^|b)",
            "g",
            "[[0,\"\",[]],[2,\"\",[]],[3,\"\",[]]]",
        ),
        ("a\n", "^", "gm", "[[0,\"\",[]]]"),
        ("\n", "^", "gm", "[[0,\"\",[]]]"),
        ("a\nb\n", "^", "gm", "[[0,\"\",[]]]"),
        ("a\nb", "^", "gm", "[[0,\"\",[]]]"),
        ("a\n\nb\n", "^", "gm", "[[0,\"\",[]]]"),
        ("a\n", "(?m:^)", "g", "[[0,\"\",[]]]"),
        ("\n", "(?m:^)", "g", "[[0,\"\",[]]]"),
        ("a\nb\n", "(?m:^)", "g", "[[0,\"\",[]],[2,\"\",[]]]"),
        ("a\nb", "(?m:^)", "g", "[[0,\"\",[]],[2,\"\",[]]]"),
        (
            "a\n\nb\n",
            "(?m:^)",
            "g",
            "[[0,\"\",[]],[2,\"\",[]],[3,\"\",[]]]",
        ),
        ("a\n", "(?m)(a|^)+", "g", "[[0,\"a\",[\"a\"]]]"),
        ("\n", "(?m)(a|^)+", "g", "[[0,\"\",[\"\"]]]"),
        (
            "a\nb\n",
            "(?m)(a|^)+",
            "g",
            "[[0,\"a\",[\"a\"]],[2,\"\",[\"\"]]]",
        ),
        (
            "a\nb",
            "(?m)(a|^)+",
            "g",
            "[[0,\"a\",[\"a\"]],[2,\"\",[\"\"]]]",
        ),
        (
            "a\n\nb\n",
            "(?m)(a|^)+",
            "g",
            "[[0,\"a\",[\"a\"]],[2,\"\",[\"\"]],[3,\"\",[\"\"]]]",
        ),
        ("a\n", "(?m)(?<=\\n)^", "g", "[]"),
        ("\n", "(?m)(?<=\\n)^", "g", "[]"),
        ("a\nb\n", "(?m)(?<=\\n)^", "g", "[[2,\"\",[]]]"),
        ("a\nb", "(?m)(?<=\\n)^", "g", "[[2,\"\",[]]]"),
        (
            "a\n\nb\n",
            "(?m)(?<=\\n)^",
            "g",
            "[[2,\"\",[]],[3,\"\",[]]]",
        ),
        ("a\n", "(?m)^\\s*$", "g", "[]"),
        ("\n", "(?m)^\\s*$", "g", "[[0,\"\\n\",[]]]"),
        ("a\nb\n", "(?m)^\\s*$", "g", "[]"),
        ("a\nb", "(?m)^\\s*$", "g", "[]"),
        ("a\n\nb\n", "(?m)^\\s*$", "g", "[[2,\"\",[]]]"),
        ("a\n", "(?m)^", "gn", "[]"),
        ("\n", "(?m)^", "gn", "[]"),
        ("a\nb\n", "(?m)^", "gn", "[]"),
        ("a\nb", "(?m)^", "gn", "[]"),
        ("a\n\nb\n", "(?m)^", "gn", "[]"),
    ] {
        assert_eq!(
            run(
                "[$s | match($p; $f) | [.offset, .string, (.captures | map(.string))]]",
                subject,
                pattern,
                flags
            )?,
            (expected.to_owned(), 0),
            "{pattern:?} ({flags:?}) on {subject:?}"
        );
    }
    Ok(())
}

/// The builtins that read a match another way (`gsub`, `test`, `splits`, `scan`) agree.
#[test]
fn the_builtins_over_a_multiline_caret_agree_6456() -> Result<()> {
    for (program, subject, pattern, flags, expected) in [
        (
            "$s | gsub($p; \"> \"; $f)",
            "a\nb\n",
            "(?m)^",
            "g",
            "\"> a\\n> b\\n\"",
        ),
        (
            "$s | gsub($p; \"> \"; $f)",
            "a\n",
            "(?m)^",
            "g",
            "\"> a\\n\"",
        ),
        (
            "$s | gsub($p; \"> \"; $f)",
            "a\nb\n",
            "(?m)\\n^",
            "g",
            "\"a> b\\n\"",
        ),
        (
            "$s | gsub($p; \"> \"; $f)",
            "a\n",
            "(?m)\\n^",
            "g",
            "\"a\\n\"",
        ),
        (
            "$s | gsub($p; \"> \"; $f)",
            "a\nb\n",
            "(?m)^$",
            "g",
            "\"a\\nb\\n\"",
        ),
        (
            "$s | gsub($p; \"> \"; $f)",
            "a\n",
            "(?m)^$",
            "g",
            "\"a\\n\"",
        ),
        ("$s | test($p; $f)", "a\nb\n", "(?m)^", "g", "true"),
        ("$s | test($p; $f)", "a\n", "(?m)^", "g", "true"),
        ("$s | test($p; $f)", "a\nb\n", "(?m)\\n^", "g", "true"),
        ("$s | test($p; $f)", "a\n", "(?m)\\n^", "g", "false"),
        ("$s | test($p; $f)", "a\nb\n", "(?m)^$", "g", "false"),
        ("$s | test($p; $f)", "a\n", "(?m)^$", "g", "false"),
        (
            "$s | sub($p; \"> \"; $f)",
            "a\nb\n",
            "(?m)^",
            "g",
            "\"> a\\n> b\\n\"",
        ),
        (
            "$s | sub($p; \"> \"; $f)",
            "a\n",
            "(?m)^",
            "g",
            "\"> a\\n\"",
        ),
        (
            "$s | sub($p; \"> \"; $f)",
            "a\nb\n",
            "(?m)\\n^",
            "g",
            "\"a> b\\n\"",
        ),
        (
            "$s | sub($p; \"> \"; $f)",
            "a\n",
            "(?m)\\n^",
            "g",
            "\"a\\n\"",
        ),
        (
            "$s | sub($p; \"> \"; $f)",
            "a\nb\n",
            "(?m)^$",
            "g",
            "\"a\\nb\\n\"",
        ),
        ("$s | sub($p; \"> \"; $f)", "a\n", "(?m)^$", "g", "\"a\\n\""),
        (
            "$s | [splits($p; $f)]",
            "a\nb\n",
            "(?m)^",
            "g",
            "[\"\",\"a\\n\",\"b\\n\"]",
        ),
        (
            "$s | [splits($p; $f)]",
            "a\n",
            "(?m)^",
            "g",
            "[\"\",\"a\\n\"]",
        ),
        (
            "$s | [splits($p; $f)]",
            "a\nb\n",
            "(?m)\\n^",
            "g",
            "[\"a\",\"b\\n\"]",
        ),
        (
            "$s | [splits($p; $f)]",
            "a\n",
            "(?m)\\n^",
            "g",
            "[\"a\\n\"]",
        ),
        (
            "$s | [splits($p; $f)]",
            "a\nb\n",
            "(?m)^$",
            "g",
            "[\"a\\nb\\n\"]",
        ),
        ("$s | [splits($p; $f)]", "a\n", "(?m)^$", "g", "[\"a\\n\"]"),
        ("$s | [scan($p; $f)]", "a\nb\n", "(?m)^", "g", "[\"\",\"\"]"),
        ("$s | [scan($p; $f)]", "a\n", "(?m)^", "g", "[\"\"]"),
        (
            "$s | [scan($p; $f)]",
            "a\nb\n",
            "(?m)\\n^",
            "g",
            "[\"\\n\"]",
        ),
        ("$s | [scan($p; $f)]", "a\n", "(?m)\\n^", "g", "[]"),
        ("$s | [scan($p; $f)]", "a\nb\n", "(?m)^$", "g", "[]"),
        ("$s | [scan($p; $f)]", "a\n", "(?m)^$", "g", "[]"),
    ] {
        assert_eq!(
            run(program, subject, pattern, flags)?,
            (expected.to_owned(), 0),
            "{program} with {pattern:?} ({flags:?}) on {subject:?}"
        );
    }
    Ok(())
}
