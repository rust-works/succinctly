//! #5367: POSIX bracket classes (`[[:alpha:]]`, `[[:upper:]]`, ...) inside a character class are
//! Unicode-aware in jq's regex engine (Oniguruma, UTF-8), so `"é" | test("[[:alpha:]]")` is `true`;
//! the `regex` crate's own are ASCII-only. Each class was also checked against the pinned jq over
//! every scalar value: whatever jq's class matches, this one does, and it matches more only for
//! code points the `regex` crate's newer Unicode tables assign (Unicode 16 against jq's older
//! Oniguruma tables). Every row below is a live jq 1.7.1 capture.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_posix_bracket_classes_are_unicode_aware_5367() -> Result<()> {
    for (input, program, want) in [
        (r#""é""#, r#"test("[[:alpha:]]")"#, "true\n"),
        (r#""É""#, r#"test("[[:upper:]]")"#, "true\n"),
        (r#""é""#, r#"test("[[:lower:]]")"#, "true\n"),
        (r#""é""#, r#"test("[[:upper:]]")"#, "false\n"),
        (r#""٣""#, r#"test("[[:digit:]]")"#, "true\n"),
        (r#""€""#, r#"test("[[:punct:]]")"#, "true\n"),
        (r#""é""#, r#"test("[[:alnum:]]")"#, "true\n"),
        (r#""é""#, r#"test("[[:word:]]")"#, "true\n"),
        (r#""\u00a0""#, r#"test("[[:space:]]")"#, "true\n"),
        (r#""\u00a0""#, r#"test("[[:blank:]]")"#, "true\n"),
        (r#""é""#, r#"test("[[:graph:]]")"#, "true\n"),
        (r#""é""#, r#"test("[[:print:]]")"#, "true\n"),
        (r#""\u00e9""#, r#"test("[[:cntrl:]]")"#, "false\n"),
        (r#""\u0085""#, r#"test("[[:cntrl:]]")"#, "true\n"),
        (r#""é""#, r#"test("[^[:alpha:]]")"#, "false\n"),
        (r#""é""#, r#"test("[[:^alpha:]]")"#, "false\n"),
        (r#""1""#, r#"test("[[:^alpha:]]")"#, "true\n"),
        (r#""é""#, r#"test("[[:alpha:][:digit:]]")"#, "true\n"),
        (
            r#""é1_""#,
            r#"[match("[[:alpha:][:digit:]_]";"g")|.string]"#,
            "[\"é\",\"1\",\"_\"]\n",
        ),
        (
            r#""日本語 abc""#,
            r#"[match("[[:alpha:]]+";"g")|.string]"#,
            "[\"日本語\",\"abc\"]\n",
        ),
        (
            r#""héllo wörld""#,
            r#"gsub("[^[:alnum:] ]";"")"#,
            "\"héllo wörld\"\n",
        ),
        (
            r#""héllo, wörld!""#,
            r#"gsub("[[:punct:]]";"")"#,
            "\"héllo wörld\"\n",
        ),
        (
            r#""a1é٣""#,
            r#"[match("[[:digit:]]";"g")|.string]"#,
            "[\"1\",\"٣\"]\n",
        ),
        (
            r#""ÀÉ àé""#,
            r#"[match("[[:upper:]]";"g")|.string]"#,
            "[\"À\",\"É\"]\n",
        ),
        (
            r#""ÀÉ àé""#,
            r#"[match("[[:lower:]]";"g")|.string]"#,
            "[\"à\",\"é\"]\n",
        ),
        (
            r#""snake_case é""#,
            r#"[match("[[:word:]]+";"g")|.string]"#,
            "[\"snake_case\",\"é\"]\n",
        ),
        (r#""]""#, r#"test("[]a[:alpha:]]")"#, "true\n"),
        (r#""é""#, r#"test("[]a[:alpha:]]")"#, "true\n"),
        (r#""é""#, r#"test("[^]a[:alpha:]]")"#, "false\n"),
        (r#""a""#, r#"test("[a\\][:alpha:]]")"#, "true\n"),
        (r#""é""#, r#"test("\\[:alpha:\\]")"#, "false\n"),
        (r#""a""#, r#"test("[:alpha:]")"#, "true\n"),
        (r#""é""#, r#"test("[[:ascii:]]")"#, "false\n"),
        (r#""a""#, r#"test("[[:ascii:]]")"#, "true\n"),
        (r#""é""#, r#"test("[[:xdigit:]]")"#, "false\n"),
        (r#""f""#, r#"test("[[:xdigit:]]")"#, "true\n"),
        (r#""abc""#, r#"test("[[:digit:]]")"#, "false\n"),
        (r#""abc""#, r#"test("[[:alpha:]]+")"#, "true\n"),
        (r#""É""#, r#"test("[[:upper:]]";"i")"#, "true\n"),
        (r#""é""#, r#"test("[[:upper:]]";"i")"#, "true\n"),
        (
            r#""Éa""#,
            r#"[match("^[[:upper:]][[:lower:]]+$").string]"#,
            "[\"Éa\"]\n",
        ),
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
