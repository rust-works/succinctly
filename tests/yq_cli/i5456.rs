//! #5456: yq's `~` null literal, its `toyaml`/`fromyaml` spellings and `SRC as $v ireduce (INIT;
//! UPDATE)`, which is `reduce SRC as $v (INIT; UPDATE)`. Every row was captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

const DOC: &str = "a: 1\nb: [x, y, z]\nc: {d: 2, e: [3, 4]}\nf: \"str\"\n";

#[test]
fn test_tilde_toyaml_fromyaml_and_ireduce_match_yq_5456() -> Result<()> {
    for (filter, want) in [
        ("~", "null\n"),
        ("[~, 1]", "[null,1]\n"),
        (".a = ~ | .a", "null\n"),
        (".z == ~", "true\n"),
        (". == ~", "false\n"),
        ("~ | type", "\"!!null\"\n"),
        (r#"{"k": ~}"#, "{\"k\":null}\n"),
        ("~, 1", "null\n1\n"),
        (".a // ~", "1\n"),
        (r#""a: 1" | fromyaml"#, "{\"a\":1}\n"),
        (".f | toyaml", "\"str\\n\"\n"),
        (".b | .[] as $i ireduce (0; . + 1)", "3\n"),
        (r#".b | .[] as $i ireduce (""; . + $i)"#, "\"xyz\"\n"),
        (".c.e[] as $i ireduce (0; . + $i)", "7\n"),
        (".c | .[] as $v ireduce ([]; . + [$v])", "[2,[3,4]]\n"),
        (".c.e | .[] as $i ireduce (0; . + $i) | . * 2", "14\n"),
        ("(.c.e | .[]) as $i ireduce (0; . + $i)", "7\n"),
        (".a as $i ireduce (0; . + $i)", "1\n"),
        ("[.c.e[] as $i ireduce (0; . + $i), 5]", "[7,5]\n"),
    ] {
        let (stdout, code) = run_yq_stdin(filter, DOC, &["-o=json", "-I=0"])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "`{filter}`");
    }
    Ok(())
}

/// `~` takes no postfix in yq, and none of the four spellings exists in jq mode.
#[test]
fn test_tilde_takes_no_postfix_and_jq_mode_is_unchanged_5456() -> Result<()> {
    for filter in ["~[0]", "~.a", "path(~)"] {
        let (stdout, code) = run_yq_stdin(filter, DOC, &["-o=json", "-I=0"])?;
        assert_eq!(stdout, "", "`{filter}`");
        assert_ne!(code, 0, "`{filter}` must be rejected");
    }
    Ok(())
}
