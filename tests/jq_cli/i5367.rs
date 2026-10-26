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
        (r#""a[b]""#, r#"[match("[[]";"g")|.string]"#, "[\"[\"]\n"),
        (
            r#""a[b]""#,
            r#"[match("[a[]";"g")|.string]"#,
            "[\"a\",\"[\"]\n",
        ),
        (r#""x[y]""#, r#"gsub("[][]";"")"#, "\"xy\"\n"),
        (
            r#""a&b""#,
            r#"[match("[a&&b]";"g")|.string]"#,
            "[\"a\",\"&\",\"b\"]\n",
        ),
        (
            r#""a&b~""#,
            r#"[match("[&~]";"g")|.string]"#,
            "[\"&\",\"~\"]\n",
        ),
        (r#""a&b""#, r#"test("[[:alpha:]&&[^a]]")"#, "false\n"),
        (r#""ABC""#, r#"test("[a&&[:alpha:]]")"#, "true\n"),
        (
            r#""é[x""#,
            r#"[match("[[:alpha:][]";"g")|.string]"#,
            "[\"é\",\"[\",\"x\"]\n",
        ),
        (
            r#""héllo wörld""#,
            r#"sub("[[:alpha:]]+";"X")"#,
            "\"X wörld\"\n",
        ),
        (
            r#""héllo wörld""#,
            r#"gsub("[[:alpha:]]+";"X")"#,
            "\"X X\"\n",
        ),
        (
            r#""héllo wörld""#,
            r#"capture("(?<w>[[:alpha:]]+) (?<v>[[:alpha:]]+)")"#,
            "{\"w\":\"héllo\",\"v\":\"wörld\"}\n",
        ),
        (
            r#""héllo wörld""#,
            r#"[splits("[^[:alpha:]]+")]"#,
            "[\"héllo\",\"wörld\"]\n",
        ),
        (
            r#""héllo wörld""#,
            r#"[scan("[[:alpha:]]+")]"#,
            "[\"héllo\",\"wörld\"]\n",
        ),
        (
            r#""héllo wörld""#,
            r#"ascii_downcase | test("^[[:alpha:] ]+$")"#,
            "true\n",
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

/// A bracket-shaped `[:name:]` inside a class whose name is not a POSIX class is an error, in
/// jq as here (the wording differs, as for every regex failure).
#[test]
fn test_unknown_posix_bracket_name_is_an_error_5367() -> Result<()> {
    for program in [
        r#"test("[[:foo:]]")"#,
        r#"test("[[:ALPHA:]]")"#,
        r#"test("[[:^foo:]]")"#,
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", program], Some(r#""é""#))?;
        assert_eq!((stdout.as_str(), code), ("", 5), "{program}: {stderr}");
        assert!(
            stderr.contains("invalid POSIX bracket type"),
            "{program}: {stderr}"
        );
    }
    Ok(())
}
