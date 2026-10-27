//! #5340: `strptime` starts from jq's zeroed `struct tm`, so a field the format leaves unset is 1900-01-00, not 1970-01-01.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

// jq clears the `struct tm` before it calls the libc (`memset(&tm, 0, sizeof(tm))`), so `tm_year = 0` (1900),
// `tm_mon = 0` and `tm_mday = 0` survive a format that does not set them, and `set_tm_wday`/`set_tm_yday` then work
// from that date, `mday = 0` included. The year, month, day and every format that sets a date field are the
// `/usr/bin/jq` 1.7.1 rows below. Weekday and yearday of a format that sets no date are the libc's (glibc leaves jq's
// sentinels 8 and 367, macOS computes from `mday = 0` with the 1900 weekday error jq's own comment admits), so the
// rows that differ there carry the date's own weekday and are recorded in limitations.md.

/// #5340: the unset year, month and day, with the weekday and yearday computed from the zeroed date.
#[test]
fn test_5340_strptime_unset_fields_are_the_zeroed_tm() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        // Agrees with the macOS pin on every field.
        (
            "null",
            r#""2015" | strptime("%Y")"#,
            "[2015,0,0,0,0,0,3,-1]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2015-03" | strptime("%Y-%m")"#,
            "[2015,2,0,0,0,0,6,58]\n",
            "",
            0,
        ),
        (
            "null",
            r#""03/05" | strptime("%m/%d")"#,
            "[1900,2,5,0,0,0,1,63]\n",
            "",
            0,
        ),
        (
            "null",
            r#""24" | strptime("%y")"#,
            "[2024,0,0,0,0,0,0,-1]\n",
            "",
            0,
        ),
        // Year, month and day agree; the weekday is the date's (macOS answers 0, 6 and 6 here).
        (
            "null",
            r#""15" | strptime("%d")"#,
            "[1900,0,15,0,0,0,1,14]\n",
            "",
            0,
        ),
        (
            "null",
            r#""10:15" | strptime("%H:%M")"#,
            "[1900,0,0,10,15,0,0,-1]\n",
            "",
            0,
        ),
        (
            "null",
            r#""Fri" | strptime("%a")"#,
            "[1900,0,0,0,0,0,0,-1]\n",
            "",
            0,
        ),
        (
            "null",
            r#""19" | strptime("%C")"#,
            "[1900,0,0,0,0,0,0,-1]\n",
            "",
            0,
        ),
        // The date and time `mktime` reads are the zeroed ones: before 1900-01-01 it refuses, as jq does.
        (
            "null",
            r#""10:15" | strptime("%H:%M") | mktime"#,
            "",
            "invalid gmtime representation",
            5,
        ),
        // A format that sets every field is unchanged.
        (
            "null",
            r#""2015-03-05T10:15:20Z" | strptime("%Y-%m-%dT%H:%M:%SZ")"#,
            "[2015,2,5,10,15,20,4,63]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2024-01-00" | strptime("%Y-%m-%d")"#,
            "[2024,0,0,0,0,0,0,-1]\n",
            "",
            0,
        ),
    ])
}
