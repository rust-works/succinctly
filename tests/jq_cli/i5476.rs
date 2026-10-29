//! #5476: jq 1.7.1 reads `-R` input with `fgets` into a 4096-byte buffer, and a multi-byte
//! character that straddles the 4095-byte boundary comes out as U+FFFD replacement characters. That corrupts
//! valid text, so `succinctly jq` keeps the character (ADR-0018, recorded in
//! `docs/compliance/jq/limitations.md`). jq's side of each row was captured from jq 1.7.1
//! (`/usr/bin/jq`): the straddling rows below print U+FFFD there, so jq's `length` is larger than
//! the character count (`主` at 4094: 4098, not 4096).

use anyhow::Result;

use crate::run_jq_stdin;

/// `prefix` ASCII bytes, `ch`, then `b`: a line whose character starts at byte offset `prefix`.
fn line(prefix: usize, ch: char) -> String {
    format!("{}{ch}b\n", "a".repeat(prefix))
}

#[test]
fn test_raw_input_keeps_a_character_straddling_the_4095_byte_read_5476() -> Result<()> {
    for (prefix, ch) in [
        // Straddling jq's read boundary: jq prints U+FFFD for these.
        (4093, '主'),
        (4094, '主'),
        (4093, '😀'),
        (4094, '😀'),
        (4094, 'é'),
        // Entirely before or after it: jq reads these intact too.
        (4092, '主'),
        (4095, '主'),
        (4095, '😀'),
        (4093, 'é'),
    ] {
        let input = line(prefix, ch);
        let want = format!("\"{}{ch}b\"\n", "a".repeat(prefix));
        let (stdout, code) = run_jq_stdin(".", &input, &["-R"])?;
        assert_eq!((stdout.as_str(), code), (want.as_str(), 0), "{prefix} {ch}");
        let (stdout, code) = run_jq_stdin("length", &input, &["-R"])?;
        let chars = prefix + 2;
        assert_eq!(
            (stdout.as_str(), code),
            (format!("{chars}\n").as_str(), 0),
            "{prefix} {ch}"
        );
    }
    Ok(())
}

#[test]
fn test_raw_input_slurp_and_null_input_keep_a_straddling_character_5476() -> Result<()> {
    let input = line(4094, '主');
    let text = format!("{}主b\n", "a".repeat(4094));
    let (stdout, code) = run_jq_stdin("length", &input, &["-Rs"])?;
    assert_eq!(
        (stdout.as_str(), code),
        (format!("{}\n", text.chars().count()).as_str(), 0)
    );
    let (stdout, code) = run_jq_stdin("input | endswith(\"主b\")", &input, &["-nR"])?;
    assert_eq!((stdout.as_str(), code), ("true\n", 0));
    Ok(())
}
