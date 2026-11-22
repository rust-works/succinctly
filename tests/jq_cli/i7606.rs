//! #7606: jq 1.8.2's `jv2tm` passes a broken-down time through `timegm` (`mktime` for
//! `strflocaltime`) before `strftime`, `strflocaltime`, `todate` and `todateiso8601` format it
//! (jqlang/jq#3203, which fills in the zone), and that call normalises the fields: out-of-range ones
//! carry over and the weekday and yearday are recomputed. Upstream never calls jq 1.7.1's
//! formatting of the fields as given a bug, so for a full 8-field array `succinctly jq` keeps it.
//! An array of fewer than 8 fields, which jq 1.7.1 refuses and #7532 reads as jq 1.8.2 does, is
//! now normalised as jq 1.8.2 normalises it (`[2024,5] | todate` was `"2024-06-00T00:00:00Z"`, a
//! date neither release prints).
//!
//! Also: `strflocaltime`'s `%s` of a wall-clock reading that is `-1` as UTC (`1969-12-31
//! 23:59:59`) is that reading's instant in the zone, where it printed `-1` (both releases agree).
//!
//! Rows were captured on macOS from jq 1.8.2 (`/opt/homebrew/bin/jq`, the short arrays, which jq
//! 1.7.1 refuses) and jq 1.7.1 (`/usr/bin/jq`, the full arrays).

use std::process::Command;

use anyhow::Result;

use crate::{spawn_with_signal_retry, succinctly_bin};

const F: &str = "%Y-%m-%dT%H:%M:%S %a %j %U %W %u %w";

/// (TZ, value, filter, stdout of `try (value | filter) catch .`), from jq 1.8.2.
const SHORT_ARRAYS: &[(&str, &str, &str, &str)] = &[
    ("UTC", "[2024,5]", "todate", "\"2024-05-31T00:00:00Z\"\n"),
    (
        "UTC",
        "[2024,5]",
        "todateiso8601",
        "\"2024-05-31T00:00:00Z\"\n",
    ),
    (
        "UTC",
        "[2024]",
        "strftime(\"%Y-%m-%d %a %j\")",
        "\"2023-12-31 Sun 365\"\n",
    ),
    (
        "UTC",
        "[2024,2,15]",
        "strftime($f)",
        "\"2024-03-15T00:00:00 Fri 075 10 11 5 5\"\n",
    ),
    (
        "UTC",
        "[2024,0,1,0,0,0,0]",
        "strftime(\"%a %j\")",
        "\"Mon 001\"\n",
    ),
    (
        "UTC",
        "[2024,0,1,0,0,60]",
        "todate",
        "\"2024-01-01T00:01:00Z\"\n",
    ),
    (
        "UTC",
        "[2024,13,35,25,61,61]",
        "strftime(\"%Y-%m-%d %H:%M:%S %a %j\")",
        "\"2025-03-08 02:02:01 Sat 067\"\n",
    ),
    (
        "UTC",
        "[2024,0,1,-1]",
        "todate",
        "\"2023-12-31T23:00:00Z\"\n",
    ),
    (
        "UTC",
        "[2024,5]",
        "strflocaltime(\"%Y-%m-%d %a %j %Z\")",
        "\"2024-05-31 Fri 152 UTC\"\n",
    ),
    (
        "Europe/Paris",
        "[2024,6,1,12]",
        "strflocaltime(\"%Y-%m-%dT%H:%M:%S %a %j %Z %z %s\")",
        "\"2024-07-01T12:00:00 Mon 183 CEST +0200 1719828000\"\n",
    ),
    // A reading the zone skipped lands past the skip; a repeated one is the earlier instant.
    (
        "Europe/Paris",
        "[2024,2,31,2,30]",
        "strflocaltime(\"%Y-%m-%dT%H:%M:%S %Z %z %s\")",
        "\"2024-03-31T03:30:00 CEST +0200 1711848600\"\n",
    ),
    (
        "Europe/Paris",
        "[2024,9,27,2,30]",
        "strflocaltime(\"%Y-%m-%dT%H:%M:%S %Z %z %s\")",
        "\"2024-10-27T02:30:00 CEST +0200 1729989000\"\n",
    ),
    (
        "America/New_York",
        "[2024,2,10,2,30]",
        "strflocaltime(\"%Y-%m-%dT%H:%M:%S %Z %z %s\")",
        "\"2024-03-10T03:30:00 EDT -0400 1710055800\"\n",
    ),
    (
        "America/New_York",
        "[2024,0,32,25]",
        "strflocaltime(\"%Y-%m-%dT%H:%M:%S %a %j %Z %z\")",
        "\"2024-02-02T01:00:00 Fri 033 EST -0500\"\n",
    ),
    (
        "Europe/Paris",
        "[2024,5]",
        "strftime(\"%Y-%m-%d %a %j %Z %z %s\")",
        "\"2024-05-31 Fri 152 UTC +0000 1717113600\"\n",
    ),
    // Before 1900 macOS's `timegm` fails and leaves the fields as given.
    (
        "UTC",
        "[1899,11,31]",
        "todate",
        "\"1899-12-31T00:00:00Z\"\n",
    ),
    (
        "UTC",
        "[]",
        "strftime(\"%Y-%m-%d %a %j\")",
        "\"1900-01-00 Sun 001\"\n",
    ),
];

/// (TZ, value, filter, stdout), from jq 1.7.1: a full array is formatted as given, where jq 1.8.2
/// normalises it (its answer in the comment), and `mktime` agrees in both.
const FULL_ARRAYS: &[(&str, &str, &str, &str)] = &[
    // 1.8.2: "Mon 001"
    (
        "UTC",
        "[2024,0,1,0,0,0,0,0]",
        "strftime(\"%a %j\")",
        "\"Sun 001\"\n",
    ),
    // 1.8.2: "2025-03-08 02:02:01"
    (
        "UTC",
        "[2024,13,35,25,61,61,0,0]",
        "strftime(\"%Y-%m-%d %H:%M:%S\")",
        "\"2024-14-35 25:61:61\"\n",
    ),
    // 1.8.2: "2024-03-01T00:00:00Z"
    (
        "UTC",
        "[2024,1,30,0,0,0,0,0]",
        "todate",
        "\"2024-02-30T00:00:00Z\"\n",
    ),
    // 1.8.2: "Mon 001"
    (
        "UTC",
        "[2024,0,1,0,0,0,3,100]",
        "strflocaltime(\"%a %j\")",
        "\"Wed 101\"\n",
    ),
    // 1.8.2: "2023-12-31T23:59:59Z"
    (
        "UTC",
        "[2024,0,1,0,0,-1,0,0]",
        "todate",
        "\"2024-01-01T00:00:-1Z\"\n",
    ),
    ("UTC", "[2024,13,35,25,61,61,0,0]", "mktime", "1741399321\n"),
    // jq 1.7.1 labels this July reading CET +0100 (jqlang/jq#1912, fixed by #3203, #3054 here);
    // jq 1.8.2 prints "Mon 183 12 CEST +0200".
    (
        "Europe/Paris",
        "[2024,6,1,12,0,0,0,0]",
        "strflocaltime(\"%a %j %H %Z %z\")",
        "\"Sun 001 12 CEST +0200\"\n",
    ),
];

/// (TZ, value, filter, stdout): `%s` of a reading that is `-1` as UTC, the same in both releases.
const WALL_CLOCK_MINUS_ONE: &[(&str, &str, &str, &str)] = &[
    (
        "Europe/Paris",
        "[1969,11,31,23,59,59,3,364]",
        "strflocaltime(\"%s\")",
        "\"-3601\"\n",
    ),
    (
        "America/New_York",
        "[1969,11,31,23,59,59,3,364]",
        "strflocaltime(\"%s\")",
        "\"17999\"\n",
    ),
    (
        "Asia/Tokyo",
        "[1970,0,1,8,59,59,4,0]",
        "strflocaltime(\"%s\")",
        "\"-1\"\n",
    ),
    (
        "UTC",
        "[1969,11,31,23,59,59,3,364]",
        "strflocaltime(\"%s\")",
        "\"-1\"\n",
    ),
    (
        "UTC",
        "[1969,11,31,23,59,59,3,364]",
        "strftime(\"%s\")",
        "\"-1\"\n",
    ),
];

fn check(rows: &[(&str, &str, &str, &str)]) -> Result<()> {
    for &(tz, value, filter, want) in rows {
        let program = format!("try ({value} | {filter}) catch .");
        let (output, code) = spawn_with_signal_retry(
            || {
                let mut command = Command::new(succinctly_bin());
                command
                    .env("TZ", tz)
                    .args(["jq", "-nc", "--arg", "f", F, &program]);
                command
            },
            None,
        )?;
        let stdout = String::from_utf8(output.stdout)?;
        assert_eq!((stdout.as_str(), code), (want, 0), "TZ={tz} {program}");
    }
    Ok(())
}

#[test]
fn short_broken_down_time_is_normalised_as_jq_1_8_2_does_7606() -> Result<()> {
    check(SHORT_ARRAYS)
}

#[test]
fn full_broken_down_time_is_formatted_as_given_as_jq_1_7_1_does_7606() -> Result<()> {
    check(FULL_ARRAYS)
}

#[test]
fn strflocaltime_s_of_a_minus_one_reading_is_its_instant_7606() -> Result<()> {
    check(WALL_CLOCK_MINUS_ONE)
}
