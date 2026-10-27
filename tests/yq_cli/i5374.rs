//! #5374: yq's `\w`, `\d`, `\s` and `\b` are Go's (RE2): ASCII classes and an ASCII word boundary, where the
//! `regex` crate's are Unicode, and `\s` has no `\v`. A `[` (and `&`, `~`) inside a class is a literal, and the
//! POSIX brackets and `\pL` are unchanged. Every row is a live yq v4.53.3 capture.

use anyhow::Result;

use crate::run_yq_stdin;

const ARGS: &[&str] = &["-n", "-o=json", "-I0"];

#[test]
fn test_yq_shorthand_classes_are_go_s_ascii_classes_5374() -> Result<()> {
    for (filter, want) in [
        ("\"é\" | test(\"\\\\w\")", "false\n"),
        ("\"٣\" | test(\"\\\\d\")", "false\n"),
        ("\" \" | test(\"\\\\s\")", "false\n"),
        ("\"\\u000b\" | test(\"\\\\s\")", "false\n"),
        ("\"é\" | test(\"\\\\b\")", "false\n"),
        ("\"é\" | test(\"\\\\W\")", "true\n"),
        ("\"٣\" | test(\"\\\\D\")", "true\n"),
        ("\" \" | test(\"\\\\S\")", "true\n"),
        ("\"a é\" | [match(\"\\\\w+\";\"g\")|.string]", "[\"a\"]\n"),
        ("\"aé b\" | [match(\"\\\\b\";\"g\")|.offset]", "[0,1,4,5]\n"),
        ("\"é\" | test(\"[\\\\w]\")", "false\n"),
        ("\"é\" | test(\"[^\\\\W]\")", "false\n"),
        ("\"é\" | test(\"[\\\\W]\")", "true\n"),
        ("\"٣\" | test(\"[\\\\d]\")", "false\n"),
        ("\" \" | test(\"[\\\\s]\")", "false\n"),
        (
            "\"a-b.c_d\" | [match(\"[\\\\w.-]+\";\"g\")|.string]",
            "[\"a-b.c_d\"]\n",
        ),
        (
            "\"a_b-c\" | [match(\"[\\\\w-]+\";\"g\")|.string]",
            "[\"a_b-c\"]\n",
        ),
        ("\"é\" | test(\"(?i)\\\\w\")", "false\n"),
        ("\"\\u212a\" | test(\"(?i)\\\\w\")", "true\n"),
        ("\"ab12\" | test(\"^\\\\w\\\\d$\")", "false\n"),
        ("\"a1\" | test(\"^\\\\w\\\\d$\")", "true\n"),
        (
            "\"日本語 abc\" | [match(\"\\\\w+\";\"g\")|.string]",
            "[\"abc\"]\n",
        ),
        ("\"x[y\" | test(\"[[]\")", "true\n"),
        ("\"[\" | test(\"[[]\")", "true\n"),
        ("\"&\" | test(\"[a&&b]\")", "true\n"),
        ("\"~\" | test(\"[~~]\")", "true\n"),
        ("\"x[y\" | test(\"[[:alpha:]]\")", "true\n"),
        ("\"é\" | test(\"[[:alpha:]]\")", "false\n"),
        ("\"é\" | test(\"\\\\pL\")", "true\n"),
        ("\"é y\" | sub(\"\\\\w\";\"X\")", "\"é X\"\n"),
        ("\"é y\" | sub(\"\\\\s\";\"_\")", "\"é_y\"\n"),
        (
            "\"1٣2\" | [match(\"\\\\d+\";\"g\")|.string]",
            "[\"1\",\"2\"]\n",
        ),
        ("\"\\\\w\" | test(\"\\\\\\\\w\")", "true\n"),
        ("\"é\" | test(\"\\\\w|é\")", "true\n"),
    ] {
        let (stdout, code) = run_yq_stdin(filter, "", ARGS)?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter}");
    }
    Ok(())
}
