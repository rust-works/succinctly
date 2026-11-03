//! #5960: `strptime` as jq 1.7.1's libc reads it (`/usr/bin/jq`, macOS): `%p` refuses an hour
//! past 12 read before it; `%z` turns the GMT flag on with an offset that a later `%s` or `GMT`
//! drops; and white space is C's `isspace`. Every row is what that jq prints under the zone it
//! names (`LC_ALL=C`).

use std::process::Command;

use anyhow::Result;

use crate::{spawn_with_signal_retry, succinctly_bin};

/// `$s | strptime($f)` under `TZ=zone` and `LC_ALL=C`: compact stdout and the exit code.
fn strptime_in_zone(zone: &str, input: &str, fmt: &str) -> Result<(String, i32)> {
    let (output, code) = spawn_with_signal_retry(
        || {
            let mut command = Command::new(succinctly_bin());
            command.env("TZ", zone).env("LC_ALL", "C").args([
                "jq",
                "-nc",
                "--arg",
                "s",
                input,
                "--arg",
                "f",
                fmt,
                "$s | strptime($f)",
            ]);
            command
        },
        None,
    )?;
    Ok((String::from_utf8(output.stdout)?, code))
}

/// Each `(zone, input, format, stdout, exit code)` row.
fn assert_rows(rows: &[(&str, &str, &str, &str, i32)]) -> Result<()> {
    for &(zone, input, fmt, want, want_code) in rows {
        let (got, code) = strptime_in_zone(zone, input, fmt)?;
        assert_eq!(
            (got.as_str(), code),
            (want, want_code),
            "TZ={zone:?} {input:?} | strptime({fmt:?})"
        );
    }
    Ok(())
}

/// #5960: `%p` refuses an hour past 12 read before it; one read after it is kept.
#[test]
fn test_5960_strptime_am_pm_refuses_an_hour_past_12() -> Result<()> {
    assert_rows(&[
        ("UTC", "2015 13 PM", "%Y %H %p", "", 5),
        ("UTC", "2015 13 AM", "%Y %H %p", "", 5),
        (
            "UTC",
            "2015 12 PM",
            "%Y %H %p",
            "[2015,0,0,12,0,0,3,-1]\n",
            0,
        ),
        (
            "UTC",
            "2015 12 AM",
            "%Y %H %p",
            "[2015,0,0,0,0,0,3,-1]\n",
            0,
        ),
        (
            "UTC",
            "2015 00 AM",
            "%Y %H %p",
            "[2015,0,0,0,0,0,3,-1]\n",
            0,
        ),
        (
            "UTC",
            "2015 11 PM",
            "%Y %H %p",
            "[2015,0,0,23,0,0,3,-1]\n",
            0,
        ),
        (
            "UTC",
            "2015 PM 13",
            "%Y %p %H",
            "[2015,0,0,13,0,0,3,-1]\n",
            0,
        ),
        ("UTC", "1425599621 PM", "%s %p", "", 5),
    ])
}

/// #5960: `%z` is a sign and four digits `hhmm`; it turns the GMT flag on and its offset comes off the UTC instant at the end, the last one read counting, unless `timegm` failed.
#[test]
fn test_5960_strptime_utc_offset_converts_to_local_time() -> Result<()> {
    assert_rows(&[
        (
            "UTC",
            "2015-03-05 10:00 +0500",
            "%F %R %z",
            "[2015,2,5,5,0,0,4,63]\n",
            0,
        ),
        (
            "UTC",
            "2015-03-05 10:00 -0130",
            "%F %R %z",
            "[2015,2,5,11,30,0,4,63]\n",
            0,
        ),
        (
            "UTC",
            "2015-03-05 10:00 -0000",
            "%F %R %z",
            "[2015,2,5,10,0,0,4,63]\n",
            0,
        ),
        (
            "UTC",
            "2015-03-05 10:00 +0060",
            "%F %R %z",
            "[2015,2,5,9,0,0,4,63]\n",
            0,
        ),
        (
            "UTC",
            "2015-03-05 10:00 +9959",
            "%F %R %z",
            "[2015,2,1,6,1,0,0,59]\n",
            0,
        ),
        ("UTC", "2015-03-05 10:00 +05", "%F %R %z", "", 5),
        ("UTC", "2015-03-05 10:00 +05:00", "%F %R %z", "", 5),
        ("UTC", "2015-03-05 10:00 0500", "%F %R %z", "", 5),
        ("UTC", "2015-03-05 10:00 Z", "%F %R %z", "", 5),
        ("UTC", "2015-03-05 10:00 +05001", "%F %R %z", "", 5),
        (
            "UTC",
            "+0500 2015-03-05 10:00",
            "%z %F %R",
            "[2015,2,5,5,0,0,4,63]\n",
            0,
        ),
        (
            "UTC",
            "2015-03-05 +0500",
            "%F %z",
            "[2015,2,4,19,0,0,3,62]\n",
            0,
        ),
        (
            "UTC",
            "2015-03-05 10:00 +0100 +0200",
            "%F %R %z %z",
            "[2015,2,5,8,0,0,4,63]\n",
            0,
        ),
        (
            "UTC",
            "1425599621 +0500",
            "%s %z",
            "[2015,2,5,18,53,41,4,63]\n",
            0,
        ),
        (
            "UTC",
            "+0500 1425599621",
            "%z %s",
            "[2015,2,5,23,53,41,4,63]\n",
            0,
        ),
        (
            "UTC",
            "2015 03 05 10 +0500 GMT",
            "%Y %m %d %H %z %Z",
            "[2015,2,5,10,0,0,4,63]\n",
            0,
        ),
        (
            "America/New_York",
            "+0500 1425599621 07",
            "%z %s %H",
            "[2015,2,5,2,53,41,4,63]\n",
            0,
        ),
        (
            "Australia/Sydney",
            "2015-03-05 10:00 +0500 AEDT",
            "%F %R %z %Z",
            "[2015,2,5,16,0,0,4,63]\n",
            0,
        ),
        (
            "UTC",
            "1425599621 +0500 1425599621",
            "%s %z %s",
            "[2015,2,5,23,53,41,4,63]\n",
            0,
        ),
        (
            "UTC",
            "2015-03-05 10:00 GMT +0500",
            "%F %R %Z %z",
            "[2015,2,5,5,0,0,4,63]\n",
            0,
        ),
        (
            "Australia/Sydney",
            "2015-03-05 10:00 +0500",
            "%F %R %z",
            "[2015,2,5,16,0,0,4,63]\n",
            0,
        ),
        (
            "America/New_York",
            "2015-03-05 10:00 +0000",
            "%F %R %z",
            "[2015,2,5,5,0,0,4,63]\n",
            0,
        ),
        (
            "UTC",
            "1900-01-01 00:00 +0100",
            "%Y-%m-%d %H:%M %z",
            "[1899,11,31,23,0,0,0,364]\n",
            0,
        ),
        (
            "UTC",
            "1899-12-31 23:00 -0200",
            "%Y-%m-%d %H:%M %z",
            "[1969,11,31,23,59,59,3,364]\n",
            0,
        ),
    ])
}

/// #5960: White space in the format, before `%s` and at the end of the input is C's `isspace`: other Unicode white space is text to match.
#[test]
fn test_5960_strptime_white_space_is_c_isspace() -> Result<()> {
    assert_rows(&[
        ("UTC", "2015\u{a0}x", "%Y x", "", 5),
        ("UTC", "2015\u{3000}x", "%Y x", "", 5),
        ("UTC", "2015\u{2003}", "%Y", "", 5),
        (
            "UTC",
            "2015 \u{85}",
            "%Y",
            "[2015,0,0,0,0,0,3,-1,\" \u{85}\"]\n",
            0,
        ),
        ("UTC", "\u{a0}1425599621", "%s", "", 5),
        ("UTC", "2015\u{b}x", "%Y x", "[2015,0,0,0,0,0,3,-1]\n", 0),
        ("UTC", "2015\u{c}x", "%Y x", "[2015,0,0,0,0,0,3,-1]\n", 0),
        ("UTC", "2015\u{d}x", "%Y x", "[2015,0,0,0,0,0,3,-1]\n", 0),
        (
            "UTC",
            "2015\u{b}",
            "%Y",
            "[2015,0,0,0,0,0,3,-1,\"\\u000b\"]\n",
            0,
        ),
        ("UTC", "\t1425599621", "%s", "[2015,2,5,23,53,41,4,63]\n", 0),
    ])
}
