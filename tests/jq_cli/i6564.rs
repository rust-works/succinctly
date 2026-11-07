//! #6564: `$ENV` and `env` panicked on a process environment holding a name or value that is not valid UTF-8
//! (`std::env::vars` unwraps it); jq 1.7.1 builds the object with its usual replacement characters, and so do
//! `$ENV.NAME` and `env.NAME`. The environment is read as bytes and substituted by the rule jq's string constructor
//! uses (`substitute_invalid_utf8_jq_style`), so the count of U+FFFD matches. Every row was captured from jq 1.7.1
//! (`env BADVAR=<bytes> jq -nc '$ENV.BADVAR | explode'`).

#![cfg(unix)]

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::process::Command;

use anyhow::Result;

fn run_with_env(name: &[u8], value: &[u8], filter: &str) -> Result<(String, String, bool)> {
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("jq")
        .arg("-nc")
        .arg(filter)
        .env(OsStr::from_bytes(name), OsStr::from_bytes(value))
        .output()?;
    Ok((
        String::from_utf8(output.stdout)?,
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.success(),
    ))
}

#[test]
fn a_non_utf8_environment_value_is_substituted_as_jq_does_6564() -> Result<()> {
    let rows: &[(&[u8], &str)] = &[
        (b"\xff\xfe", "[65533,65533]"),
        (b"\x80", "[65533]"),
        (b"\x61\xe2\x82", "[97,65533]"),
        (b"\xc0\x80", "[65533,65533]"),
        (b"\xed\xa0\x80", "[65533]"),
        (b"\xf4\x90\x80\x80", "[65533]"),
        (b"\xf0\x41\x42", "[65533]"),
        (b"\x6f\x6b\xe1\x41", "[111,107,65533]"),
        (b"\xe1", "[65533]"),
        (b"\x70\x6c\x61\x69\x6e", "[112,108,97,105,110]"),
        (b"\xe2\x82\xac\xff\xe2\x82\xac", "[8364,65533,8364]"),
    ];
    for &(value, expected) in rows {
        for filter in [
            "$ENV.BADVAR | explode",
            "env.BADVAR | explode",
            "$ENV | .BADVAR | explode",
        ] {
            let (stdout, stderr, ok) = run_with_env(b"BADVAR", value, filter)?;
            assert!(ok, "{value:?} {filter}: {stderr}");
            assert_eq!(stdout.trim_end(), expected, "{value:?} {filter}");
        }
        // Building the whole object must not panic either, whichever variable it holds.
        let (stdout, stderr, ok) = run_with_env(b"BADVAR", value, "$ENV | length > 0")?;
        assert!(ok, "{value:?}: {stderr}");
        assert_eq!(stdout.trim_end(), "true");
    }
    Ok(())
}

/// A name that is not valid UTF-8 is substituted the same way (jq: `BAD\u{fffd}NAME`).
#[test]
fn a_non_utf8_environment_name_is_substituted_as_jq_does_6564() -> Result<()> {
    for filter in [
        "$ENV | keys | map(select(startswith(\"B\"))) | map(explode)",
        "env | keys | map(select(startswith(\"B\"))) | map(explode)",
    ] {
        let (stdout, stderr, ok) = run_with_env(b"BAD\xffNAME", b"1", filter)?;
        assert!(ok, "{filter}: {stderr}");
        assert_eq!(
            stdout.trim_end(),
            "[[66,65,68,65533,78,65,77,69]]",
            "{filter}"
        );
    }
    Ok(())
}
