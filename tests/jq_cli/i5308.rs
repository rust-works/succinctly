//! #5308: a string with an invalid escape (`"\x"`) or a lone high surrogate (`"\ud800"`) is not
//! rejected up front. jq 1.7.1 refuses both at parse time (`Invalid escape`, `Invalid
//! \uXXXX\uXXXX surrogate pair escape`, exit 2) whatever the filter; succinctly semi-indexes, so a
//! filter that only forwards the value echoes its source span and a filter that reads it raises
//! (exit 5). That is the "a filter validates only what it reads" rule of #2103 and #2168, applied to
//! a string value. These pin the split, so the entry in `docs/compliance/jq/limitations.md` stays
//! true, and that `--validate` still rejects both.

use anyhow::Result;

use crate::{names_fault, run_jq_full};

#[test]
fn test_forwarding_filter_echoes_a_string_with_an_undecodable_escape_5308() -> Result<()> {
    for input in [r#""\x""#, r#""\ud800""#, r#""\ud800A""#, r#"["\x"]"#] {
        for filter in [".", ".,.", "select(.)"] {
            let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(input))?;
            assert_eq!(code, 0, "`{filter}` over {input}: {stderr}");
            assert!(
                stdout.starts_with(input),
                "`{filter}` over {input} echoes its source span, got {stdout:?}"
            );
        }
    }
    // `-s` forwards the same span inside its array; `-r` echoes it with its quotes, since nothing
    // decoded it.
    for (args, want) in [
        (&["-c", "-s", "."][..], "[\"\\x\"]\n"),
        (&["-r", "."][..], "\"\\x\"\n"),
    ] {
        let (stdout, stderr, code) = run_jq_full(args, Some(r#""\x""#))?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{args:?}: {stderr}");
    }
    Ok(())
}

#[test]
fn test_reading_filter_raises_on_a_string_with_an_undecodable_escape_5308() -> Result<()> {
    for (input, want) in [
        (r#""\x""#, "invalid escape sequence"),
        (r#""\ud800""#, "invalid unicode escape"),
    ] {
        for filter in ["length", "tostring", "tojson", ". == \"x\""] {
            let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(input))?;
            assert_eq!(code, 5, "`{filter}` over {input}: {stdout:?} {stderr}");
            assert!(
                names_fault(&stderr, want),
                "`{filter}` over {input} should say {want:?}, got {stderr:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn test_validate_rejects_a_string_with_an_undecodable_escape_5308() -> Result<()> {
    for input in [r#""\x""#, r#""\ud800""#, r#"{"a":"\ud800"}"#] {
        let (stdout, stderr, code) = run_jq_full(&["--validate", "-c", "."], Some(input))?;
        assert_eq!(code, 3, "--validate over {input}: {stdout:?} {stderr}");
        assert_eq!(stdout, "", "--validate prints nothing for {input}");
    }
    Ok(())
}
