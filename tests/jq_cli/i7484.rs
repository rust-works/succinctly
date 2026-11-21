//! #7484: `now` is a whole number of microseconds, as jq's is (`tv_sec + tv_usec / 1000000.0` from
//! `gettimeofday`, in jq 1.7.1 and 1.8.2). `succinctly jq` took the nanoseconds a Linux clock has
//! (`Duration::as_secs_f64`), so about three readings in four printed a seventh decimal that jq never
//! prints. A macOS clock has microseconds already, so this holds there before the fix too; the unit test
//! beside `unix_seconds_in_microseconds` fails on any platform without it.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn now_is_a_whole_number_of_microseconds_7484() -> Result<()> {
    // The distance of `now * 1e6` from a whole number, over 300 readings: jq 1.7.1 and 1.8.2 print `true`.
    let (out, err, code) = run_jq_full(
        &[
            "-n",
            "[range(300) | now * 1e6 | . - round | fabs] | max < 0.001",
        ],
        None,
    )?;
    assert_eq!((out.as_str(), err.as_str(), code), ("true\n", "", 0));

    // No reading prints more than six decimals.
    let (out, err, code) = run_jq_full(
        &[
            "-n",
            "[range(300) | now | tostring | select(test(\"[.][0-9]{7,}\"))] | length",
        ],
        None,
    )?;
    assert_eq!((out.as_str(), err.as_str(), code), ("0\n", "", 0));
    Ok(())
}
