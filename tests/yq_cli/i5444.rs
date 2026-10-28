//! #5444: yq's lexer has no bare `env`, `@text` or `@html` token (v4.53.3: `invalid input text`),
//! so `succinctly yq` takes them only under `--jq-extensions`. `env(NAME)` and the formats yq
//! does have stay available without the flag.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

#[test]
fn test_bare_env_text_and_html_need_jq_extensions_5444() -> Result<()> {
    let plain = &["-o=json", "-I=0"];
    let extended = &["--jq-extensions", "-o=json", "-I=0"];
    for (filter, expected) in [
        ("env | type", "\"!!map\"\n"),
        ("[env] | length", "1\n"),
        (".a | env | type", "\"!!map\"\n"),
        (".a | @text", "\"1\"\n"),
        ("@text | type", "\"!!str\"\n"),
        (".a | tostring | @html", "\"1\"\n"),
        (r#""<&>" | @html"#, "\"&lt;&amp;&gt;\"\n"),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, "a: 1\n", plain)?;
        assert_eq!(stdout, "", "`{filter}` must print nothing without the flag");
        assert_ne!(code, 0, "`{filter}` must be rejected without the flag");
        assert!(
            stderr.contains("--jq-extensions"),
            "`{filter}` error must name the flag: {stderr:?}"
        );
        assert_eq!(
            run_yq_stdin_with_stderr(filter, "a: 1\n", extended)?,
            (expected.into(), String::new(), 0),
            "--jq-extensions `{filter}`"
        );
    }
    Ok(())
}

/// What yq has stays flag-free: `env(NAME)`, `strenv(NAME)` and its own formats.
#[test]
fn test_yq_native_env_and_formats_need_no_flag_5444() -> Result<()> {
    let plain = &["-o=json", "-I=0"];
    for (filter, expected) in [
        ("env(PATH) | type", "\"!!str\"\n"),
        ("strenv(PATH) | type", "\"!!str\"\n"),
        (".a | @json", "\"1\"\n"),
        ("[.a] | @csv", "\"1\"\n"),
        (".a | tostring | @base64", "\"MQ==\"\n"),
        (".a | tostring | @uri", "\"1\"\n"),
        ("@yaml", "\"a: 1\\n\"\n"),
        ("@props", "\"a = 1\\n\"\n"),
    ] {
        assert_eq!(
            run_yq_stdin_with_stderr(filter, "a: 1\n", plain)?,
            (expected.into(), String::new(), 0),
            "`{filter}`"
        );
    }
    Ok(())
}
