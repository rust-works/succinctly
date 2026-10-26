//! #5373: jq's `\w` (Oniguruma) is its Unicode `word` plus six Latin-1 characters (U+00B2, U+00B3,
//! U+00B9, U+00BC-U+00BE) outside a character class, and without the two joiners (U+200C, U+200D)
//! the `regex` crate's `\w` adds; inside a class it is the plain Unicode word. `\W` is the
//! complement. Checked over every scalar value against the pinned jq: nothing jq matches is
//! missed, and the only extra matches are code points the `regex` crate's newer Unicode tables
//! assign. Every row is a live jq 1.7.1 capture.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_backslash_w_is_oniguruma_s_word_class_5373() -> Result<()> {
    for (input, program, want) in [
        (r#""²""#, r#"test("\\w")"#, "true\n"),
        (r#""½""#, r#"test("\\w")"#, "true\n"),
        (r#""\u200c""#, r#"test("\\w")"#, "false\n"),
        (r#""\u200d""#, r#"test("\\w")"#, "false\n"),
        (r#""²""#, r#"test("\\W")"#, "false\n"),
        (r#""é""#, r#"test("\\w")"#, "true\n"),
        (r#""a""#, r#"test("\\W")"#, "false\n"),
        (r#""x²y""#, r#"[splits("\\W")]"#, "[\"x²y\"]\n"),
        (r#""a²b""#, r#"[match("\\w+";"g")|.string]"#, "[\"a²b\"]\n"),
        (r#""a²""#, r#"gsub("\\w";"x")"#, "\"xx\"\n"),
        (
            r#""¹²³ ¼½¾""#,
            r#"[match("\\w+";"g")|.string]"#,
            "[\"¹²³\",\"¼½¾\"]\n",
        ),
        (r#""²""#, r#"test("[\\w]")"#, "false\n"),
        (r#""²""#, r#"test("[^\\W]")"#, "false\n"),
        (r#""²""#, r#"test("[\\W]")"#, "true\n"),
        (
            r#""a-b.c_d""#,
            r#"[match("[\\w.-]+";"g")|.string]"#,
            "[\"a-b.c_d\"]\n",
        ),
        (
            r#""a_b-c""#,
            r#"[match("[\\w-]+";"g")|.string]"#,
            "[\"a_b-c\"]\n",
        ),
        (r#""a1""#, r#"test("^\\w\\d$")"#, "true\n"),
        (r#""\\w""#, r#"test("\\\\w")"#, "true\n"),
        (
            r#""日本語 abc""#,
            r#"[match("\\w+";"g")|.string]"#,
            "[\"日本語\",\"abc\"]\n",
        ),
        (r#""A_9""#, r#"test("^\\w+$")"#, "true\n"),
        (r#""a b""#, r#"[splits("\\W+")]"#, "[\"a\",\"b\"]\n"),
        (r#""²""#, r#"test("\\w";"i")"#, "true\n"),
        (
            r#""A²""#,
            r#"[match("\\w";"gi")|.string]"#,
            "[\"A\",\"²\"]\n",
        ),
        (r#""a b""#, r#"test("a\\ \\w";"x")"#, "true\n"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", program], Some(input))?;
        assert_eq!(
            (stdout.as_str(), code),
            (want, 0),
            "{program} on {input}: {stderr}"
        );
    }
    Ok(())
}
