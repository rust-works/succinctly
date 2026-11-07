//! #5930: `from_unix` names the tag of a non-number in yq mode only; jq mode keeps its own wording (jq 1.7.1 has no
//! `from_unix`, so this is succinctly's extension), and a date-like string times a number is jq's string repetition.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn from_unix_and_a_date_like_string_in_arithmetic_are_unchanged_in_jq_mode_5930() -> Result<()> {
    let (_stdout, stderr, status) = run_jq_full(&["-n", r#""x" | from_unix"#], None)?;
    assert_ne!(status, 0);
    assert!(stderr.contains("math function requires number"), "{stderr}");
    let (stdout, _, status) = run_jq_full(&["-nc", r#""2026-01-01T00:00:00Z" * 2"#], None)?;
    assert_eq!(
        (stdout.trim_end(), status),
        (r#""2026-01-01T00:00:00Z2026-01-01T00:00:00Z""#, 0)
    );
    Ok(())
}
