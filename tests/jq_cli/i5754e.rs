//! #5754: `strptime`'s `%Z` reads a run of capital letters naming `GMT` or one of the local zone's
//! `tzname` names (the latest standard and daylight-time names it has had), and nothing else.
//! `GMT`, like `%s`, turns the libc's GMT flag on: the fields are then a UTC instant, normalised as
//! `timegm` does, and become that instant's local time once the whole format is read. `timegm`
//! fails before 1900, giving -1. Every row is what jq 1.7.1 (`/usr/bin/jq`, macOS) prints under the
//! zone it names.

use std::process::Command;

use anyhow::Result;

use crate::{spawn_with_signal_retry, succinctly_bin};

/// `$s | strptime($f)` under `TZ=zone`: compact stdout and the exit code.
fn strptime_in_zone(zone: &str, input: &str, fmt: &str) -> Result<(String, i32)> {
    let (output, code) = spawn_with_signal_retry(
        || {
            let mut command = Command::new(succinctly_bin());
            command.env("TZ", zone).args([
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

/// #5754: `%Z` takes a run of capital letters: `GMT`, or one of the local zone's `tzname` names, the latest standard and daylight-time ones it has had. Any other name, lower case, an empty run and `UTC` outside UTC are no match; under an empty `TZ` only `GMT` is.
#[test]
fn test_5754_strptime_zone_name_is_gmt_or_a_local_tzname() -> Result<()> {
    assert_rows(&[
        (
            "Australia/Sydney",
            "2015-03-05 AEDT",
            "%F %Z",
            "[2015,2,5,0,0,0,4,63]\n",
            0,
        ),
        (
            "Australia/Sydney",
            "2015-03-05 AEST",
            "%F %Z",
            "[2015,2,5,0,0,0,4,63]\n",
            0,
        ),
        ("Australia/Sydney", "2015-03-05 UTC", "%F %Z", "", 5),
        ("Australia/Sydney", "2015-03-05 EST", "%F %Z", "", 5),
        (
            "Australia/Brisbane",
            "2015-03-05 AEDT",
            "%F %Z",
            "[2015,2,5,0,0,0,4,63]\n",
            0,
        ),
        (
            "Pacific/Honolulu",
            "2015-03-05 HST",
            "%F %Z",
            "[2015,2,5,0,0,0,4,63]\n",
            0,
        ),
        (
            "Pacific/Honolulu",
            "2015-03-05 HPT",
            "%F %Z",
            "[2015,2,5,0,0,0,4,63]\n",
            0,
        ),
        ("Pacific/Honolulu", "2015-03-05 HDT", "%F %Z", "", 5),
        (
            "Asia/Tokyo",
            "2015-03-05 JDT",
            "%F %Z",
            "[2015,2,5,0,0,0,4,63]\n",
            0,
        ),
        (
            "America/New_York",
            "2015-03-05 EDT",
            "%F %Z",
            "[2015,2,5,0,0,0,4,63]\n",
            0,
        ),
        (
            "EST5EDT",
            "2015-03-05 EDT",
            "%F %Z",
            "[2015,2,5,0,0,0,4,63]\n",
            0,
        ),
        (
            "XYZ3",
            "2015-03-05 XYZ",
            "%F %Z",
            "[2015,2,5,0,0,0,4,63]\n",
            0,
        ),
        (
            "UTC",
            "2015-03-05 UTC",
            "%F %Z",
            "[2015,2,5,0,0,0,4,63]\n",
            0,
        ),
        ("UTC", "2015-03-05 utc", "%F %Z", "", 5),
        ("UTC", "2015-03-05 Z", "%F %Z", "", 5),
        ("UTC", "2015-03-05 ", "%F %Z", "", 5),
        ("UTC", "2015-03-05 GMTX", "%F %Z", "", 5),
        ("UTC", "2015-03-05 GMT+1", "%F %Z", "", 5),
        (
            "UTC",
            "2015-03-05 GMT x",
            "%F %Z",
            "[2015,2,5,0,0,0,4,63,\" x\"]\n",
            0,
        ),
        ("UTC", "2015-03-05GMT", "%F%Z", "[2015,2,5,0,0,0,4,63]\n", 0),
        ("", "2015-03-05 UTC", "%F %Z", "", 5),
        ("", "2015-03-05 GMT", "%F %Z", "[2015,2,5,0,0,0,4,63]\n", 0),
    ])
}

/// #5754: `GMT` makes the fields a UTC instant that becomes local time at the end, after every other field, normalised as `timegm` does; a zone's own name changes nothing.
#[test]
fn test_5754_strptime_gmt_converts_to_local_time() -> Result<()> {
    assert_rows(&[
        (
            "Australia/Sydney",
            "2015-03-05 10:00 GMT",
            "%F %R %Z",
            "[2015,2,5,21,0,0,4,63]\n",
            0,
        ),
        (
            "America/New_York",
            "2015-03-05 10:00 GMT",
            "%F %R %Z",
            "[2015,2,5,5,0,0,4,63]\n",
            0,
        ),
        (
            "Europe/London",
            "2015-07-05 10:00 GMT",
            "%F %R %Z",
            "[2015,6,5,11,0,0,0,185]\n",
            0,
        ),
        (
            "Europe/London",
            "2015-07-05 10:00 BST",
            "%F %R %Z",
            "[2015,6,5,10,0,0,0,185]\n",
            0,
        ),
        (
            "Asia/Kolkata",
            "2015-03-05 GMT",
            "%F %Z",
            "[2015,2,5,5,30,0,4,63]\n",
            0,
        ),
        (
            "Australia/Sydney",
            "GMT 2015-03-05",
            "%Z %F",
            "[2015,2,5,11,0,0,4,63]\n",
            0,
        ),
        (
            "Australia/Sydney",
            "2015-12-31 23:30 GMT",
            "%F %R %Z",
            "[2016,0,1,10,30,0,5,0]\n",
            0,
        ),
        (
            "America/New_York",
            "2015 01 09 GMT",
            "%Y %m %U %Z",
            "[2015,1,28,19,0,0,6,58]\n",
            0,
        ),
        (
            "Australia/Sydney",
            "2015 GMT",
            "%Y %Z",
            "[2014,11,31,11,0,0,3,364]\n",
            0,
        ),
        (
            "Australia/Sydney",
            "2015-03-05 GMT AEDT",
            "%F %Z %Z",
            "[2015,2,5,11,0,0,4,63]\n",
            0,
        ),
        (
            "Australia/Sydney",
            "2015-03-05 GMT",
            "%F %EZ",
            "[2015,2,5,11,0,0,4,63]\n",
            0,
        ),
    ])
}

/// #5754: `timegm` fails before 1900-01-01 00:00 UTC and returns -1, so the zeroed 1900-01-00 (`%Z` alone) and an earlier `%s` are 1969-12-31 23:59:59 UTC in local time.
#[test]
fn test_5754_strptime_gmt_before_1900_is_the_instant_before_the_epoch() -> Result<()> {
    assert_rows(&[
        ("UTC", "GMT", "%Z", "[1969,11,31,23,59,59,3,364]\n", 0),
        (
            "Australia/Sydney",
            "10 GMT",
            "%H %Z",
            "[1970,0,1,9,59,59,4,0]\n",
            0,
        ),
        (
            "UTC",
            "1900-03-01 00:00 GMT",
            "%Y-%m-%d %H:%M %Z",
            "[1900,2,1,0,0,0,4,59]\n",
            0,
        ),
        (
            "UTC",
            "1899-12-31 23:59:59 GMT",
            "%F %T %Z",
            "[1969,11,31,23,59,59,3,364]\n",
            0,
        ),
        ("UTC", "-2203891200", "%s", "[1900,2,1,0,0,0,4,59]\n", 0),
        (
            "UTC",
            "-2208988801",
            "%s",
            "[1969,11,31,23,59,59,3,364]\n",
            0,
        ),
        (
            "Australia/Sydney",
            "-3000000000",
            "%s",
            "[1970,0,1,9,59,59,4,0]\n",
            0,
        ),
    ])
}

/// #5754: `%s` sets the fields in UTC and turns the GMT flag on, so a field read after it is a UTC field.
#[test]
fn test_5754_strptime_epoch_seconds_reach_local_time_after_the_fields() -> Result<()> {
    assert_rows(&[
        (
            "Australia/Sydney",
            "1425599621",
            "%s",
            "[2015,2,6,10,53,41,5,64]\n",
            0,
        ),
        (
            "Australia/Sydney",
            "1425599621 05",
            "%s %H",
            "[2015,2,5,16,53,41,4,63]\n",
            0,
        ),
        (
            "America/New_York",
            "1425599621 2016",
            "%s %Y",
            "[2016,2,5,18,53,41,6,64]\n",
            0,
        ),
        (
            "Australia/Sydney",
            "1425599621 30",
            "%s %M",
            "[2015,2,6,10,30,41,5,64]\n",
            0,
        ),
        (
            "Australia/Sydney",
            "05 1425599621",
            "%H %s",
            "[2015,2,6,10,53,41,5,64]\n",
            0,
        ),
        (
            "Australia/Sydney",
            "1425599621 GMT",
            "%s %Z",
            "[2015,2,6,10,53,41,5,64]\n",
            0,
        ),
    ])
}
