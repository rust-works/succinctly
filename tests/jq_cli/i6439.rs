//! #6439: a counted repeat of a Unicode class is refused past the `regex` crate's size limit, where jq answers
//! it. The crate writes the repeat out copy by copy; the limit is 128 MiB (the crate's default 10 MiB refused
//! `\w{1000}`), and a higher one costs gigabytes (`docs/compliance/jq/limitations.md`). The refusal is in jq's
//! words and the repeats that fit are answered. Every `false` was captured from jq 1.7.1 (`jq -nc`), and jq
//! answers the refused row too.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_counted_repeat_of_a_unicode_class_past_the_size_limit_is_refused_in_jq_s_words_6439(
) -> Result<()> {
    let pattern = r"\w{3000}";
    let (stdout, _, status) = run_jq_full(
        &["-nc", "--arg", "p", pattern, r#""a" | try test($p) catch ."#],
        None,
    )?;
    assert_eq!(
        (stdout.trim_end(), status),
        (
            r#""Regex failure: Compiled regex exceeds size limit of 134217728 bytes.""#,
            0
        ),
        "{pattern}"
    );
    Ok(())
}

#[test]
fn a_counted_repeat_that_fits_the_size_limit_is_answered_6439() -> Result<()> {
    for pattern in [
        "a{100000}",
        "[a-z]{100000}",
        ".{100000}",
        "(?:a|b){100000}",
        r"\w{1000}",
        r"(?:\pL{1000}){100}",
    ] {
        let (stdout, _, status) = run_jq_full(
            &[
                "-nc",
                "--arg",
                "p",
                pattern,
                r#""a" | try test($p) catch ."#,
            ],
            None,
        )?;
        assert_eq!((stdout.trim_end(), status), ("false", 0), "{pattern}");
    }
    Ok(())
}
