//! #7615: a hexadecimal integer literal in a yq filter (`0x10`, `0X10`) is yq's `HexValue` token,
//! `0[xX][0-9a-fA-F]+`, read with `strconv.ParseInt(digits, 16, 64)` and kept as the node's text.
//! So it is the integer a document's `0x10` is (#2802): 16 for arithmetic, its spelling for
//! `tostring`, `==`, `unique` and a YAML write. Every row is a live yq v4.53.3 capture.
//! jq has no hex literal, and `succinctly jq` still refuses one.

use anyhow::Result;

use crate::{run_jq_stdin_with_stderr, run_yq_stdin, run_yq_stdin_with_stderr};

#[test]
fn test_hex_literal_matches_yq_7615() -> Result<()> {
    for (filter, want) in [
        ("0x10", "16"),
        ("0x10 | tostring", "\"0x10\""),
        ("[0x1f, 31] | unique", "[31,31]"),
        ("[0x1f, 0x1f] | unique", "[31]"),
        ("0x10 | tag", "\"!!int\""),
        ("0x7fffffffffffffff + 1", "-9223372036854775808"),
        ("0x7fffffffffffffff", "9223372036854775807"),
        ("0x7fffffffffffffff | tostring", "\"0x7fffffffffffffff\""),
        ("0X10", "16"),
        ("0X10 | tostring", "\"0X10\""),
        ("0x0", "0"),
        ("0x00 | tostring", "\"0x00\""),
        ("0xAbC", "2748"),
        ("0xAbC | tostring", "\"0xAbC\""),
        ("0x010", "16"),
        ("0x010 | tostring", "\"0x010\""),
        ("0x10a", "266"),
        ("(0x10)", "16"),
        ("[0x10]", "[16]"),
        ("{\"a\": 0x10}", "{\"a\":16}"),
        ("{\"a\": 0x10} | .a | tostring", "\"0x10\""),
        ("\"\\(0x10)\"", "\"0x10\""),
        ("0x10 + 1", "17"),
        ("1 - 0x10", "-15"),
        ("0x10 * 2", "32"),
        ("\"a\" * 0x2", "\"aa\""),
        ("0x10 == 16", "false"),
        ("0x10 == 0x10", "true"),
        ("0x10 == 0X10", "false"),
        ("0x10 == 0x010", "false"),
        ("0x10 < 17", "true"),
        ("0x10 | length", "4"),
        ("0x10 as $x | $x", "16"),
        ("[1,2,3] | .[0x1]", "2"),
        ("[1,2,3] | .[0x1:]", "[2,3]"),
        ("[1,2,3] | .[:0x2]", "[1,2]"),
        ("[0x2, 0x1] | sort", "[1,2]"),
        ("[0x1, 0x2] | .[] |= tostring", "[\"0x1\",\"0x2\"]"),
        ("0x10 | to_yaml", "\"0x10\\n\""),
        ("\"x\" + 0x10", "\"x0x10\""),
    ] {
        let (out, code) = run_yq_stdin(filter, "", &["-n", "-o=json", "-I0"])?;
        assert_eq!((out.trim_end(), code), (want, 0), "{filter}");
    }
    Ok(())
}

/// A write keeps the literal's spelling, and `==` reads it as a document's spelled integer does.
#[test]
fn test_hex_literal_writes_and_compares_like_a_document_integer_7615() -> Result<()> {
    for (filter, input, want) in [
        (".a = 0x10", "a: 16\n", "a: 0x10\n"),
        (".a == 0x10", "a: 16\n", "false\n"),
        (".a == 0x10", "a: 0x10\n", "true\n"),
        (
            ".b = 0x10 | .c = .a",
            "a: 0x10\n",
            "a: 0x10\nb: 0x10\nc: 0x10\n",
        ),
        (".a += 0x10", "a: 1\n", "a: 17\n"),
        (
            ".a = [0x10, {\"b\": 0X1F}]",
            "{}\n",
            "a:\n  - 0x10\n  - b: 0X1F\n",
        ),
    ] {
        let (out, code) = run_yq_stdin(filter, input, &[])?;
        assert_eq!((out.as_str(), code), (want, 0), "{filter} over {input:?}");
    }
    Ok(())
}

/// The token has no sign, `_`, fraction or empty digit run, and a value past `i64` is Go's error.
#[test]
fn test_hex_literal_refusals_match_yq_7615() -> Result<()> {
    for filter in [
        "-0x10", "[-0x10]", "0x10.a", "0x", "0xg", "0x_1", "0x1.8", "00x10", "0x10x",
    ] {
        let (out, _, code) = run_yq_stdin_with_stderr(filter, "", &["-n", "--"])?;
        assert_eq!((out.as_str(), code), ("", 1), "{filter}");
    }
    for (filter, digits) in [
        ("0xFFFFFFFFFFFFFFFF", "FFFFFFFFFFFFFFFF"),
        ("0x8000000000000000", "8000000000000000"),
        ("1, 0x8000000000000000", "8000000000000000"),
    ] {
        let (out, err, code) = run_yq_stdin_with_stderr(filter, "", &["-n"])?;
        assert_eq!((out.as_str(), code), ("", 1), "{filter}");
        let want = format!("strconv.ParseInt: parsing \"{digits}\": value out of range");
        assert!(err.contains(&want), "{filter}: {err}");
    }
    Ok(())
}

/// jq has no hex literal (jq 1.7.1: `syntax error, unexpected IDENT`, exit 3); jq mode is unchanged.
#[test]
fn test_jq_mode_still_refuses_a_hex_literal_7615() -> Result<()> {
    for filter in ["0x10", "[0X1f]"] {
        let (out, _, code) = run_jq_stdin_with_stderr(filter, "", &["-n"])?;
        assert_eq!((out.as_str(), code), ("", 3), "{filter}");
    }
    Ok(())
}
