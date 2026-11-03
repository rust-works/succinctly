//! #6052: a JSON number written with a trailing dot and no fraction (`5.`, `99999999999999999.`) is
//! read by jq everywhere it reads JSON, as the integer: `fromjson`, `--argjson`, `tonumber` and the
//! document reader. `fromjson` and `--argjson` refused it (rule 4c, #2240) because the dot went to a
//! lossy `f64` fallback, which turned a large integer into the wrong one
//! (`99999999999999999.` was `100000000000000000`). The dot is dropped before the literal is kept now.
//! Every row is what `/usr/bin/jq` 1.7.1 printed for the same program.

use anyhow::Result;

use crate::{run_jq_full, run_jq_stdin};

/// `-nc --arg s TEXT PROGRAM`: stdout of a run that must succeed.
fn on_string(text: &str, program: &str) -> Result<String> {
    let (stdout, stderr, code) = run_jq_full(&["-nc", "--arg", "s", text, program], None)?;
    assert_eq!(code, 0, "`{program}` on {text:?} failed: {stderr}");
    Ok(stdout.trim().to_string())
}

const ROWS: &[(&str, &str)] = &[
    ("1.", "1"),
    ("-1.", "-1"),
    ("0.", "0"),
    ("-0.", "-0"),
    ("007.", "7"),
    ("+5.", "5"),
    ("1. ", "1"),
    ("99999999999999999.", "99999999999999999"),
    ("-99999999999999999.", "-99999999999999999"),
    ("100000000000000000000.", "100000000000000000000"),
    ("[1.]", "[1]"),
    ("{\"a\":1.}", "{\"a\":1}"),
    ("[1.,2.,99999999999999999.]", "[1,2,99999999999999999]"),
];

#[test]
fn fromjson_reads_a_trailing_dot_as_the_integer() -> Result<()> {
    for (text, want) in ROWS {
        assert_eq!(
            on_string(text, "$s | fromjson")?,
            *want,
            "fromjson {text:?}"
        );
    }
    Ok(())
}

#[test]
fn the_document_reader_reads_a_trailing_dot_as_the_integer() -> Result<()> {
    for (text, want) in ROWS {
        let (out, code) = run_jq_stdin(".", text, &["-c"])?;
        assert_eq!(code, 0, "document {text:?}");
        assert_eq!(out.trim(), *want, "document {text:?}");
    }
    Ok(())
}

#[test]
fn argjson_reads_a_trailing_dot_as_the_integer() -> Result<()> {
    for (text, want) in ROWS {
        let (stdout, stderr, code) = run_jq_full(&["-nc", "--argjson", "s", text, "$s"], None)?;
        assert_eq!(code, 0, "--argjson {text:?}: {stderr}");
        assert_eq!(stdout.trim(), *want, "--argjson {text:?}");
    }
    Ok(())
}

#[test]
fn tonumber_keeps_the_integer_digits_of_a_trailing_dot() -> Result<()> {
    // Only the bare numbers: `tonumber` of `[1.]` is jq's "cannot be parsed as a number".
    for (text, want) in ROWS.iter().filter(|(t, _)| !t.starts_with(['[', '{'])) {
        assert_eq!(
            on_string(text, "$s | tonumber")?,
            *want,
            "tonumber {text:?}"
        );
    }
    Ok(())
}

#[test]
fn a_trailing_dot_still_has_to_follow_an_integer_part() -> Result<()> {
    // jq refuses each of these in all three readers.
    for text in [".", "-.", "1.5.", "1..", "1.x", "[1.x]"] {
        let (_, _, code) = run_jq_full(&["-nc", "--arg", "s", text, "$s | fromjson"], None)?;
        assert_ne!(code, 0, "fromjson {text:?}");
        let (_, _, code) = run_jq_full(&["-nc", "--argjson", "s", text, "$s"], None)?;
        assert_ne!(code, 0, "--argjson {text:?}");
        let (_, code) = run_jq_stdin(".", text, &["-c"])?;
        assert_ne!(code, 0, "document {text:?}");
    }
    Ok(())
}

#[test]
fn arithmetic_on_a_trailing_dot_number_rounds_like_jq() -> Result<()> {
    // The literal is exact; a computed value is a double, as in jq.
    assert_eq!(
        on_string("14455058590201385605.", "$s | fromjson | . + 0")?,
        "14455058590201387000"
    );
    assert_eq!(
        on_string(
            "99999999999999999.",
            "[$s | fromjson, ($s | tonumber)] | map(. + 0)"
        )?,
        "[1e+17,1e+17]"
    );
    Ok(())
}
