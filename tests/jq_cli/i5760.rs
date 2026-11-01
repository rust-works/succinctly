//! #5760: a `JQ_COLORS` value that is not UTF-8 is rejected like a malformed spec: jq prints
//! `Failed to set $JQ_COLORS` on stderr and keeps the default colours. It used to be ignored
//! silently. Expected stdout and stderr captured from jq 1.7.1 (`/usr/bin/jq`).

#![cfg(unix)]

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;

use anyhow::Result;

use crate::spawn_jq_with_env;

#[test]
fn test_a_non_utf8_jq_colors_warns_and_keeps_defaults_5760() -> Result<()> {
    for value in [&b"\xff"[..], b"1:\xff", b"\xe9", b"0;31:\xc3\x28"] {
        let (output, code) = spawn_jq_with_env(
            &["-C", "-c", "."],
            "JQ_COLORS",
            OsStr::from_bytes(value),
            Some(b"1"),
        )?;
        assert_eq!(code, 0, "{value:?}");
        assert_eq!(
            String::from_utf8(output.stdout)?,
            "\x1b[0;39m1\x1b[0m\n",
            "{value:?}"
        );
        assert_eq!(
            String::from_utf8(output.stderr)?,
            "Failed to set $JQ_COLORS\n",
            "{value:?}"
        );
    }
    // A valid UTF-8 spec with a non-ASCII field is the same rejection, as before.
    let (output, _) = spawn_jq_with_env(&["-C", "-c", "."], "JQ_COLORS", "1:é", Some(b"1"))?;
    assert_eq!(
        String::from_utf8(output.stderr)?,
        "Failed to set $JQ_COLORS\n"
    );
    Ok(())
}
