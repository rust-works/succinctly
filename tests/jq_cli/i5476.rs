//! #5476: jq 1.7.1 reads `-R` input with `fgets` into a 4096-byte buffer, and a multi-byte
//! character that straddles the 4095-byte boundary comes out as U+FFFD replacement characters. That corrupts
//! valid text, so `succinctly jq` keeps the character (ADR-0018, recorded in
//! `docs/compliance/jq/limitations.md`). jq's side of each row was captured from jq 1.7.1
//! (`/usr/bin/jq`): the straddling rows below print U+FFFD there, so jq's `length` is larger than
//! the character count (`主` at 4094: 4098, not 4096).
//!
//! jq 1.8.2 keeps the character too (jqlang/jq#3389, fixed by jqlang/jq#3421): the last test pins
//! the output it prints on each route, which is a `newer-than-primary` difference from 1.7.1.

use anyhow::Result;

use crate::{run_jq_full, run_jq_stdin};

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

/// jq 1.8.2's output (`/opt/homebrew/bin/jq`) for the line of jqlang/jq#3389's shape: `é` at byte
/// 4094, which jq 1.7.1 prints as two U+FFFD on every route here but `--rawfile` (`"aaaa��éé"`).
#[test]
fn every_raw_route_keeps_a_straddling_character_as_jq_1_8_2_does_5476() -> Result<()> {
    let input = format!("{}ééé\n", "a".repeat(4094));
    for (args, want) in [
        (&["-R", ".[4090:]"][..], "\"aaaaééé\"\n"),
        (&["-R", "-s", ".[4090:]"][..], "\"aaaaééé\\n\"\n"),
        (&["-n", "-R", "input[4090:]"][..], "\"aaaaééé\"\n"),
        (
            &["-n", "-R", "-c", "[inputs[4090:]]"][..],
            "[\"aaaaééé\"]\n",
        ),
        (
            &["-n", "--rawfile", "x", "/dev/stdin", "$x[4090:]"][..],
            "\"aaaaééé\\n\"\n",
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(args, Some(&input))?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (want, "", 0),
            "{args:?}"
        );
    }
    Ok(())
}
