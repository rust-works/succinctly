//! #6331: `%I`, `%l` and `%r` print the hour modulo 12 (12 for a multiple of 12), with C's sign, for an
//! hour outside 0-23 in a hand-built broken-down time. Every row was captured from jq 1.7.1
//! (`/usr/bin/jq -nc`).

use anyhow::Result;

use crate::run_jq_full;

fn assert_rows(rows: &[(&str, &str)]) -> Result<()> {
    for (filter, expected) in rows {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(
            (stdout.trim_end(), code),
            (*expected, 0),
            "{filter}: stderr {stderr}"
        );
    }
    Ok(())
}

#[test]
fn test_the_twelve_hour_clock_is_the_hour_modulo_twelve_6331() -> Result<()> {
    assert_rows(&[
        (
            r#"[2015,2,5,25,0,0,4,63] | strftime("%I|%l|%r|%p")"#,
            r#""01| 1|01:00:00 PM|PM""#,
        ),
        (
            r#"[2015,2,5,36,0,0,4,63] | strftime("%I|%l|%r|%p")"#,
            r#""12|12|12:00:00 PM|PM""#,
        ),
        (
            r#"[2015,2,5,100,0,0,4,63] | strftime("%I|%l|%r|%p")"#,
            r#""04| 4|04:00:00 PM|PM""#,
        ),
        (
            r#"[2015,2,5,-13,0,0,4,63] | strftime("%I|%l|%r|%p")"#,
            r#""-1|-1|-1:00:00 AM|AM""#,
        ),
    ])
}

/// What already agreed: the 24 hours of a day.
#[test]
fn test_the_hours_of_a_day_keep_their_twelve_hour_form_6331() -> Result<()> {
    assert_rows(&[(
        r#"[0,12,13,23] | map([2015,2,5,.,0,0,4,63] | strftime("%I%p")) | join(",")"#,
        r#""12AM,12PM,01PM,11PM""#,
    )])
}
