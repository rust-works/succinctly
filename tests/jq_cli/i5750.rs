//! #5750, #5751: `JQ_COLORS` is parsed as jq 1.7.1's `jq_set_colors` does: a field over its
//! 12-byte colour buffer (or one that is not digits and `;`) rejects the whole variable, and the
//! field loop ends when the text left after a `:` is empty. Every expected byte string was
//! captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::spawn_jq_with_env;

#[test]
fn test_jq_colors_is_parsed_like_jq_set_colors_5750() -> Result<()> {
    let input = r#"[null,true,1,{"a":false}]"#;
    // (spec, stdout, whether jq warned `Failed to set $JQ_COLORS`)
    let rows: &[(&str, &str, bool)] = &[
        ("", "\x1b[1;39m[\x1b[0;90mnull\x1b[0m\x1b[1;39m,\x1b[0;39mtrue\x1b[0m\x1b[1;39m,\x1b[0;39m1\x1b[0m\x1b[1;39m,\x1b[1;39m{\x1b[0m\x1b[1;34m\"a\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[0;39mfalse\x1b[0m\x1b[1;39m\x1b[1;39m}\x1b[0m\x1b[1;39m\x1b[1;39m]\x1b[0m\n", false),
        (":::", "\x1b[1;39m[\x1b[mnull\x1b[0m\x1b[1;39m,\x1b[mtrue\x1b[0m\x1b[1;39m,\x1b[0;39m1\x1b[0m\x1b[1;39m,\x1b[1;39m{\x1b[0m\x1b[1;34m\"a\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[mfalse\x1b[0m\x1b[1;39m\x1b[1;39m}\x1b[0m\x1b[1;39m\x1b[1;39m]\x1b[0m\n", false),
        ("1:", "\x1b[1;39m[\x1b[1mnull\x1b[0m\x1b[1;39m,\x1b[0;39mtrue\x1b[0m\x1b[1;39m,\x1b[0;39m1\x1b[0m\x1b[1;39m,\x1b[1;39m{\x1b[0m\x1b[1;34m\"a\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[0;39mfalse\x1b[0m\x1b[1;39m\x1b[1;39m}\x1b[0m\x1b[1;39m\x1b[1;39m]\x1b[0m\n", false),
        (":", "\x1b[1;39m[\x1b[mnull\x1b[0m\x1b[1;39m,\x1b[0;39mtrue\x1b[0m\x1b[1;39m,\x1b[0;39m1\x1b[0m\x1b[1;39m,\x1b[1;39m{\x1b[0m\x1b[1;34m\"a\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[0;39mfalse\x1b[0m\x1b[1;39m\x1b[1;39m}\x1b[0m\x1b[1;39m\x1b[1;39m]\x1b[0m\n", false),
        ("0;31:::::::", "\x1b[m[\x1b[0;31mnull\x1b[0m\x1b[m,\x1b[mtrue\x1b[0m\x1b[m,\x1b[m1\x1b[0m\x1b[m,\x1b[m{\x1b[0m\x1b[1;34m\"a\"\x1b[0m\x1b[m:\x1b[0m\x1b[mfalse\x1b[0m\x1b[m\x1b[m}\x1b[0m\x1b[m\x1b[m]\x1b[0m\n", false),
        ("123456789012", "\x1b[1;39m[\x1b[123456789012mnull\x1b[0m\x1b[1;39m,\x1b[0;39mtrue\x1b[0m\x1b[1;39m,\x1b[0;39m1\x1b[0m\x1b[1;39m,\x1b[1;39m{\x1b[0m\x1b[1;34m\"a\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[0;39mfalse\x1b[0m\x1b[1;39m\x1b[1;39m}\x1b[0m\x1b[1;39m\x1b[1;39m]\x1b[0m\n", false),
        ("1;2;3;4;5;6;7;8;9", "\x1b[1;39m[\x1b[0;90mnull\x1b[0m\x1b[1;39m,\x1b[0;39mtrue\x1b[0m\x1b[1;39m,\x1b[0;39m1\x1b[0m\x1b[1;39m,\x1b[1;39m{\x1b[0m\x1b[1;34m\"a\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[0;39mfalse\x1b[0m\x1b[1;39m\x1b[1;39m}\x1b[0m\x1b[1;39m\x1b[1;39m]\x1b[0m\n", true),
        ("1234567890123", "\x1b[1;39m[\x1b[0;90mnull\x1b[0m\x1b[1;39m,\x1b[0;39mtrue\x1b[0m\x1b[1;39m,\x1b[0;39m1\x1b[0m\x1b[1;39m,\x1b[1;39m{\x1b[0m\x1b[1;34m\"a\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[0;39mfalse\x1b[0m\x1b[1;39m\x1b[1;39m}\x1b[0m\x1b[1;39m\x1b[1;39m]\x1b[0m\n", true),
        ("1:1234567890123", "\x1b[1;39m[\x1b[0;90mnull\x1b[0m\x1b[1;39m,\x1b[0;39mtrue\x1b[0m\x1b[1;39m,\x1b[0;39m1\x1b[0m\x1b[1;39m,\x1b[1;39m{\x1b[0m\x1b[1;34m\"a\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[0;39mfalse\x1b[0m\x1b[1;39m\x1b[1;39m}\x1b[0m\x1b[1;39m\x1b[1;39m]\x1b[0m\n", true),
        ("1:2:3:4:5:6:7:8:1234567890123", "\x1b[6m[\x1b[1mnull\x1b[0m\x1b[6m,\x1b[3mtrue\x1b[0m\x1b[6m,\x1b[4m1\x1b[0m\x1b[6m,\x1b[7m{\x1b[0m\x1b[8m\"a\"\x1b[0m\x1b[7m:\x1b[0m\x1b[2mfalse\x1b[0m\x1b[7m\x1b[7m}\x1b[0m\x1b[6m\x1b[6m]\x1b[0m\n", false),
    ];
    for &(spec, want, warned) in rows {
        let (output, code) = spawn_jq_with_env(
            &["-C", "-c", "."],
            "JQ_COLORS",
            spec,
            Some(input.as_bytes()),
        )?;
        assert_eq!(code, 0, "JQ_COLORS={spec:?}");
        assert_eq!(
            String::from_utf8(output.stdout)?,
            want,
            "JQ_COLORS={spec:?}"
        );
        let stderr = String::from_utf8(output.stderr)?;
        assert_eq!(
            stderr.contains("Failed to set $JQ_COLORS"),
            warned,
            "JQ_COLORS={spec:?}: {stderr}"
        );
    }
    Ok(())
}
