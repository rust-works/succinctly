//! #6439: a counted repeat of a Unicode class was refused past the `regex` crate's size limit on
//! the earlier engine, where jq answers it. The crate writes the repeat out copy by copy; the limit
//! was 128 MiB (the crate's default 10 MiB refused `\w{1000}`). That was a recorded divergence; on
//! Oniguruma the row past the limit answers as jq 1.7.1 does (#7965). Every `false` was captured
//! from jq 1.7.1 (`jq -nc`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_counted_repeat_of_a_unicode_class_past_the_size_limit_is_answered_6439() -> Result<()> {
    let pattern = r"\w{3000}";
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
