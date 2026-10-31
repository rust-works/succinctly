//! #5654: `--argjson`, `--jsonargs` and `--slurpfile` skip only the four bytes jq's lexer does
//! (space, tab, CR, LF) around a value, not a form feed, a vertical tab or a Unicode space, and
//! `--argjson ''` is refused rather than `null`. Every expectation was captured from jq 1.7.1
//! (`/usr/bin/jq`); stderr wording differs and is not compared.

use anyhow::Result;
use std::io::Write;

use crate::run_jq_full;

#[test]
fn test_argjson_and_jsonargs_refuse_what_jq_refuses_around_a_value_5654() -> Result<()> {
    for text in [
        "1\u{c}",
        "\u{c}1",
        "1\u{b}",
        "\u{b}1",
        "1\u{a0}",
        "\u{a0}1",
        "\u{2003}1",
        "[1,\u{c}2]",
        "\u{c}",
        "",
        "  ",
        "\u{feff}",
        "\u{feff} ",
    ] {
        for args in [
            ["-nc", "--argjson", "x", text, "$x"].as_slice(),
            ["-nc", "--jsonargs", "$ARGS.positional", text].as_slice(),
        ] {
            let (stdout, _stderr, code) = run_jq_full(args, None)?;
            assert_eq!((stdout.as_str(), code), ("", 2), "{args:?}");
        }
    }
    Ok(())
}

#[test]
fn test_argjson_and_jsonargs_still_skip_json_whitespace_and_one_bom_5654() -> Result<()> {
    for (text, as_variable, as_positional) in [
        (" 1 ", "1\n", "[1]\n"),
        ("\t\r\n1\r\n", "1\n", "[1]\n"),
        ("\u{feff}1", "1\n", "[1]\n"),
        ("\u{feff} 1", "1\n", "[1]\n"),
        ("null", "null\n", "[null]\n"),
        ("[1]", "[1]\n", "[[1]]\n"),
    ] {
        let (stdout, _stderr, code) = run_jq_full(&["-nc", "--argjson", "x", text, "$x"], None)?;
        assert_eq!(
            (stdout.as_str(), code),
            (as_variable, 0),
            "--argjson {text:?}"
        );
        let (stdout, _stderr, code) =
            run_jq_full(&["-nc", "--jsonargs", "$ARGS.positional", text], None)?;
        assert_eq!(
            (stdout.as_str(), code),
            (as_positional, 0),
            "--jsonargs {text:?}"
        );
    }
    Ok(())
}

/// `--slurpfile` over a file holding `contents`: stdout and exit status.
fn slurpfile(contents: &str) -> Result<(String, i32)> {
    let mut file = tempfile::NamedTempFile::new()?;
    write!(file, "{contents}")?;
    let path = file.path().to_string_lossy().into_owned();
    let (stdout, _stderr, code) = run_jq_full(&["-nc", "--slurpfile", "x", &path, "$x"], None)?;
    Ok((stdout, code))
}

#[test]
fn test_slurpfile_refuses_what_jq_refuses_around_a_value_5654() -> Result<()> {
    for contents in ["1\u{c}", "\u{b}1 ", "1\u{a0}", "\u{a0}1", "\u{2003}1"] {
        assert_eq!(slurpfile(contents)?, (String::new(), 2), "{contents:?}");
    }
    Ok(())
}

#[test]
fn test_slurpfile_still_skips_json_whitespace_and_one_bom_5654() -> Result<()> {
    for (contents, expected) in [
        (" \t\r\n[1] [2]\r\n", "[[1],[2]]\n"),
        ("\u{feff} 1", "[1]\n"),
        ("", "[]\n"),
        ("  \n", "[]\n"),
    ] {
        assert_eq!(
            slurpfile(contents)?,
            (expected.to_string(), 0),
            "{contents:?}"
        );
    }
    Ok(())
}
