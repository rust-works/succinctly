//! #7429: `modulemeta` reads a module's raw bytes as `import` does (#5351).
//!
//! jq parses the bytes of a module, so a byte that is not UTF-8 is harmless in a comment, reads as
//! U+FFFD in a string (a metadata value included) and is a syntax error anywhere else. `modulemeta`
//! refused the whole file (`stream did not contain valid UTF-8`, answer `null`) where `import`
//! decodes it lossily. Every row is the answer of `/usr/bin/jq` 1.7.1
//! (`jq -nc -L lib '"u" | modulemeta'`, exit 0).

use anyhow::Result;

use crate::run_jq_in_dir;

fn modulemeta(source: &[u8]) -> Result<(String, String, i32)> {
    let dir = tempfile::tempdir()?;
    std::fs::create_dir_all(dir.path().join("lib"))?;
    std::fs::write(dir.path().join("lib/u.jq"), source)?;
    run_jq_in_dir(
        dir.path(),
        &["-nc", "-L", "lib", r#""u" | modulemeta"#],
        None,
    )
}

#[test]
fn test_modulemeta_decodes_a_module_lossily_7429() -> Result<()> {
    for (source, want) in [
        (&b"def f: \"\xff\";"[..], "{\"deps\":[],\"defs\":[\"f/0\"]}\n"),
        (b"# \xff\ndef f: 1;", "{\"deps\":[],\"defs\":[\"f/0\"]}\n"),
        (
            b"def f: 1; # \xfe\xff\ndef g: 2;",
            "{\"deps\":[],\"defs\":[\"f/0\",\"g/0\"]}\n",
        ),
        // A truncated multi-byte sequence in a string.
        (b"def f: \"\xe2\x82\";", "{\"deps\":[],\"defs\":[\"f/0\"]}\n"),
        // In the header's metadata the byte reads as U+FFFD, as in a string.
        (
            b"module {a:\"\xff\"}; def f: 1;",
            "{\"a\":\"\u{fffd}\",\"deps\":[],\"defs\":[\"f/0\"]}\n",
        ),
        (
            b"import \"x\" as x {search:\"\xff\"}; def f: 1;",
            "{\"deps\":[{\"search\":\"\u{fffd}\",\"as\":\"x\",\"is_data\":false,\"relpath\":\"x\"}],\"defs\":[\"f/0\"]}\n",
        ),
    ] {
        let (stdout, stderr, code) = modulemeta(source)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (want, "", 0),
            "{source:?}"
        );
    }
    Ok(())
}

/// A byte outside a comment or string is a syntax error, as in jq: the answer is `null` with a
/// diagnostic (its wording past `jq: error: ` is succinctly's own), not the UTF-8 refusal.
#[test]
fn test_modulemeta_invalid_byte_outside_a_string_is_a_syntax_error_7429() -> Result<()> {
    let (stdout, stderr, code) = modulemeta(b"def f: 1; \xff def g: 2;")?;
    assert_eq!((stdout.as_str(), code), ("null\n", 0), "{stderr}");
    assert!(stderr.starts_with("jq: error: "), "{stderr}");
    assert!(!stderr.contains("valid UTF-8"), "{stderr}");
    Ok(())
}
