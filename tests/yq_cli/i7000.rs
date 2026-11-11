//! #7000: in yq mode, decoded bytes that are not UTF-8 become one U+FFFD per byte, Go's count, which
//! is what yq's JSON encoder writes for the raw bytes yq keeps. Every row was captured from yq v4.53.3
//! (`yq -p json -o json`); yq escapes U+FFFD as `\ufffd` and `succinctly yq` writes the character
//! (the #1616 divergence), so the rows compare the decoded string.

use anyhow::Result;

use std::io::Write;

use crate::{run_yq_files, run_yq_stdin};

/// (filter, the JSON text of `.s`, the decoded string yq prints).
const ROWS: &[(&str, &str, &str)] = &[
    (".s | @base64d", r#""4oI=""#, "\u{FFFD}\u{FFFD}"),
    (".s | @base64d", r#""4oKs/w==""#, "\u{20AC}\u{FFFD}"),
    (".s | @base64d", r#""8J+Y""#, "\u{FFFD}\u{FFFD}\u{FFFD}"),
    (".s | @base64d", r#""wL8=""#, "\u{FFFD}\u{FFFD}"),
    (".s | @base64d", r#""7aCA""#, "\u{FFFD}\u{FFFD}\u{FFFD}"),
    (".s | @urid", r#""a%E2%82""#, "a\u{FFFD}\u{FFFD}"),
    (".s | @urid", r#""%FF%FE""#, "\u{FFFD}\u{FFFD}"),
    (".s | @urid", r#""%E2%82%AC""#, "\u{20AC}"),
    (".s | @urid", r#""%ED%A0%80""#, "\u{FFFD}\u{FFFD}\u{FFFD}"),
];

#[test]
fn decoded_bytes_that_are_not_utf8_are_one_replacement_each_7000() -> Result<()> {
    for &(filter, text, decoded) in ROWS {
        let (out, code) = run_yq_stdin(
            filter,
            &format!("{{\"s\": {text}}}"),
            &["-p", "json", "-o", "json"],
        )?;
        let printed: String = serde_json::from_str(&out)?;
        assert_eq!((printed.as_str(), code), (decoded, 0), "{filter} on {text}");
    }
    Ok(())
}

/// `load_str` decodes its file with the same rule: `x`, `e2 82`, `y` is `"x\ufffd\ufffdy"` in yq.
#[test]
fn load_str_writes_one_replacement_per_invalid_byte_7000() -> Result<()> {
    let mut file = tempfile::NamedTempFile::new()?;
    file.write_all(b"x\xe2\x82y")?;
    let filter = format!("load_str({:?})", file.path().display().to_string());
    let (out, _, code) = run_yq_files(&filter, &[], &["-n", "-o", "json"])?;
    let printed: String = serde_json::from_str(&out)?;
    assert_eq!((printed.as_str(), code), ("x\u{FFFD}\u{FFFD}y", 0));
    Ok(())
}
