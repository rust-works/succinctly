//! #6699: yq's path-element token takes the opening quote of a dotted name and stops at the first
//! space, so `."a b"` is `."a` followed by text no rule takes: `lexer: invalid input text`. Every
//! other character, a tab and a newline included, is inside the token, and a name with no space
//! is read as before. An empty name is not unquoted: `.""` is the key of two quote characters.
//! Every answer was captured from yq v4.53.3 (`-o=json -I=0`).

use anyhow::Result;

use crate::run_yq_stdin;

const DOC: &str = "{\"a b\": 1, \" a\": 2, \"a \": 3, \"\": 4, \"x\": {\"a b\": 5, \"y z\": 6, \"\": 5}, \"a  b\": 7, \"p q\": {\"r\": 8}, \"ab\": 9, \"a\\tb\": 10, \"c.d\": 11, \"y\": 1}\n";

#[test]
fn test_yq_refuses_a_space_in_a_quoted_dotted_name_6699() -> Result<()> {
    for filter in [
        ".\"a b\"",
        ".x.\"a b\"",
        ".x.\"y z\"",
        ".\" a\"",
        ".\"a \"",
        ".\"a  b\"",
        ".\"p q\".r",
        ".\"p q\" | .r",
        ".\"a b\"?",
        ".\"a b\" // 1",
        ".. | .\"a b\"?",
        ".\"a b\" = 9",
        "del(.\"a b\")",
        "(.\"a b\") |= 10",
        ".x | .\"a b\"",
        ".x.\"a b\" as $v | $v",
        ".\"a b\".c",
    ] {
        let (stdout, code) = run_yq_stdin(filter, DOC, &["-o=json", "-I=0"])?;
        assert_eq!((stdout.as_str(), code), ("", 1), "{filter}");
    }
    Ok(())
}

#[test]
fn test_yq_reads_the_other_quoted_dotted_names_and_the_empty_key_6699() -> Result<()> {
    for (filter, want) in [
        (".\"ab\"", "9\n"),
        (".\"a\\tb\"", "null\n"),
        (".\"c.d\"", "11\n"),
        (".\"y\"", "1\n"),
        (".[\" a\"]", "2\n"),
        (".x[\"a b\"]", "5\n"),
        (".[ \"a b\" ]", "1\n"),
        (".\"\"", "null\n"),
        (".x.\"\"", "null\n"),
        (".\"\"?", "null\n"),
        (". | .\"\"", "null\n"),
        (".\"\" = 9", "{\"a b\":1,\" a\":2,\"a \":3,\"\":4,\"x\":{\"a b\":5,\"y z\":6,\"\":5},\"a  b\":7,\"p q\":{\"r\":8},\"ab\":9,\"a\\tb\":10,\"c.d\":11,\"y\":1,\"\\\"\\\"\":9}\n"),
        (".x.\"\" = 9", "{\"a b\":1,\" a\":2,\"a \":3,\"\":4,\"x\":{\"a b\":5,\"y z\":6,\"\":5,\"\\\"\\\"\":9},\"a  b\":7,\"p q\":{\"r\":8},\"ab\":9,\"a\\tb\":10,\"c.d\":11,\"y\":1}\n"),
        ("del(.\"\")", "{\"a b\":1,\" a\":2,\"a \":3,\"\":4,\"x\":{\"a b\":5,\"y z\":6,\"\":5},\"a  b\":7,\"p q\":{\"r\":8},\"ab\":9,\"a\\tb\":10,\"c.d\":11,\"y\":1}\n"),
        (".x | .\"\"", "null\n"),
        (".\"\" | length", "0\n"),
        (".x | has(\"\")", "true\n"),
        ("(.x.\"\"), .y", "null\n1\n"),
    ] {
        let (stdout, code) = run_yq_stdin(filter, DOC, &["-o=json", "-I=0"])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter}");
    }
    Ok(())
}

/// The message names the text after the spaces, quoted as Go's `%q` does (yq v4.53.3 prints
/// `lexer: invalid input text "b\""` for `."a b"`).
#[test]
fn test_yq_names_the_text_after_the_spaces_in_the_lexer_error_6699() -> Result<()> {
    use crate::run_yq_stdin_with_stderr;
    for (filter, text) in [
        (r#"."a b""#, r#""b\"""#),
        (r#".x."a b""#, r#""b\"""#),
        (r#"."a  b""#, r#""b\"""#),
        (r#"." a""#, r#""a\"""#),
        (r#"."a ""#, r#""\"""#),
        (r#"."p q".r"#, r#""q\".r""#),
        (r#"."a b" = 9"#, r#""b\" = 9""#),
        (r#"."a b"#, r#""b""#),
        (r#"."a b" # c"#, r#""b\" # c""#),
        // Go's `%q` writes a carriage return and the other ASCII controls as escapes.
        (".\"a b\r\"", r#""b\r\"""#),
        (".\"a b\x01\"", r#""b\x01\"""#),
        (".\"a b\x7f\"", r#""b\x7f\"""#),
        (".\"a b\x1b[0m\"", r#""b\x1b[0m\"""#),
        (".\"a b\t\u{e9}\"", "\"b\\t\u{e9}\\\"\""),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, "a: 1\n", &[])?;
        assert_eq!((stdout.as_str(), code), ("", 1), "{filter}");
        assert!(
            stderr.contains(&format!("lexer: invalid input text {text}")),
            "{filter}: {stderr}"
        );
    }
    Ok(())
}
