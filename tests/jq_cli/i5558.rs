//! #5558: without `-C`, jq still applies `-S` and `-a` to the line `debug`/`debug(msg)` write to
//! stderr (it drops only the indent flags). The line ignored both. Every expected byte string was
//! captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

fn debug_stderr(args: &[&str], filter: &str) -> Result<String> {
    let mut all = vec!["-n"];
    all.extend_from_slice(args);
    all.push(filter);
    let (_, stderr, code) = run_jq_full(&all, None)?;
    assert_eq!(code, 0, "{filter}: {stderr}");
    Ok(stderr)
}

const VALUE: &str = r#"{"b":[1,null,{"z":true,"a":"é"}],"a":1}"#;

#[test]
fn test_debug_line_honors_sort_keys_and_ascii_5558() -> Result<()> {
    let program = format!("{VALUE} | debug | empty");
    for (args, expected) in [
        (
            &["-S"][..],
            "[\"DEBUG:\",{\"a\":1,\"b\":[1,null,{\"a\":\"é\",\"z\":true}]}]\n",
        ),
        (
            &["-a"],
            "[\"DEBUG:\",{\"b\":[1,null,{\"z\":true,\"a\":\"\\u00e9\"}],\"a\":1}]\n",
        ),
        (
            &["-S", "-a"],
            "[\"DEBUG:\",{\"a\":1,\"b\":[1,null,{\"a\":\"\\u00e9\",\"z\":true}]}]\n",
        ),
        // The indent flags never reach the line.
        (
            &["-S", "--tab"],
            "[\"DEBUG:\",{\"a\":1,\"b\":[1,null,{\"a\":\"é\",\"z\":true}]}]\n",
        ),
        (
            &["-a", "--indent", "5"],
            "[\"DEBUG:\",{\"b\":[1,null,{\"z\":true,\"a\":\"\\u00e9\"}],\"a\":1}]\n",
        ),
    ] {
        assert_eq!(debug_stderr(args, &program)?, expected, "{args:?}");
    }
    Ok(())
}

/// `debug(msg)` writes one line per output of `msg`, each with the flags applied.
#[test]
fn test_debug_msg_lines_honor_sort_keys_and_ascii_5558() -> Result<()> {
    assert_eq!(
        debug_stderr(
            &["-S", "-a"],
            r#"{"y":1,"x":"é"} | debug("é", "b") | empty"#
        )?,
        "[\"DEBUG:\",\"\\u00e9\"]\n[\"DEBUG:\",\"b\"]\n"
    );
    Ok(())
}

/// A NaN, a literal past a double and a trailing zero are spelled as jq spells them on stdout.
#[test]
fn test_debug_line_with_flags_keeps_number_spelling_5558() -> Result<()> {
    assert_eq!(
        debug_stderr(
            &["-S"],
            r#"{"n":nan,"big":100000000000000000000000,"f":1.50} | debug | empty"#
        )?,
        "[\"DEBUG:\",{\"big\":100000000000000000000000,\"f\":1.50,\"n\":null}]\n"
    );
    Ok(())
}

/// A character past the BMP is a surrogate pair under `-a`, a literal past a double keeps its
/// exponent spelling, and the flags that change only stdout (`-r`, `-j`, `--seq`) leave the
/// line alone.
#[test]
fn test_debug_line_with_flags_matches_jq_on_wide_input_5558() -> Result<()> {
    let program = r#"["😀",1e1000,-0,1E2,0.10] | debug | empty"#;
    for args in [&["-a"][..], &["-a", "-r"], &["-a", "-j"], &["-a", "--seq"]] {
        assert_eq!(
            debug_stderr(args, program)?,
            "[\"DEBUG:\",[\"\\ud83d\\ude00\",1E+1000,-0,1E+2,0.10]]\n",
            "{args:?}"
        );
    }
    // With `-C` the same text is colored, so removing the escapes gives back the plain line.
    let colored = debug_stderr(&["-C", "-a", "-S"], program)?;
    let mut plain = String::new();
    let mut chars = colored.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for e in chars.by_ref() {
                if e == 'm' {
                    break;
                }
            }
        } else {
            plain.push(c);
        }
    }
    assert_eq!(
        plain,
        "[\"DEBUG:\",[\"\\ud83d\\ude00\",1E+1000,-0,1E+2,0.10]]\n"
    );
    Ok(())
}

/// With neither flag the line is as it was, `-C` still colors it, and `-M` still turns that off.
#[test]
fn test_debug_line_without_flags_is_unchanged_5558() -> Result<()> {
    let program = format!("{VALUE} | debug | empty");
    for args in [&[][..], &["-c"], &["-M"], &["--tab"]] {
        assert_eq!(
            debug_stderr(args, &program)?,
            "[\"DEBUG:\",{\"b\":[1,null,{\"z\":true,\"a\":\"é\"}],\"a\":1}]\n",
            "{args:?}"
        );
    }
    // `-M` after `-C` wins, and `-S` still applies to the uncolored line.
    assert_eq!(
        debug_stderr(&["-C", "-M", "-S"], &program)?,
        "[\"DEBUG:\",{\"a\":1,\"b\":[1,null,{\"a\":\"é\",\"z\":true}]}]\n"
    );
    Ok(())
}
