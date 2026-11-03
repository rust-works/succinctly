//! #5947: `strptime` reads empty fields and signed years as jq 1.7.1's libc does (`/usr/bin/jq`,
//! macOS): `%Y`, `%y`, `%M` and `%S` read nothing at the end of the input or before white space,
//! `%s` with no digits reads 0, and `%Y`, `%y` and `%C` take a sign. Every row is what that jq
//! prints under the zone it names (`LC_ALL=C`), with the weekday (index 6) dropped: the zeroed
//! date's weekday (#5340) and a negative year's (#970) are recorded divergences, not this issue.

use std::process::Command;

use anyhow::Result;

use crate::{spawn_with_signal_retry, succinctly_bin};

/// `$s | strptime($f) | del(.[6])` under `TZ=zone` and `LC_ALL=C`: compact stdout and the exit
/// code.
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
                "$s | strptime($f) | del(.[6])",
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

/// #5947: `%Y`, `%y`, `%M` and `%S` (also with `%E`/`%O`, and inside `%R`, `%T`, `%D`, `%x`, `%X`) read nothing at the end of the input or before white space; the field stays as it was and does not count as read. Every other specifier refuses.
#[test]
fn test_5947_strptime_an_empty_field_reads_nothing() -> Result<()> {
    assert_rows(&[
        ("UTC", "", "%Y", "[1900,0,0,0,0,0,-1]\n", 0),
        ("UTC", " ", "%Y", "[1900,0,0,0,0,0,-1,\" \"]\n", 0),
        ("UTC", "\t", "%EY", "[1900,0,0,0,0,0,-1,\"\\t\"]\n", 0),
        ("UTC", "", "%y", "[1900,0,0,0,0,0,-1]\n", 0),
        ("UTC", "", "%M", "[1900,0,0,0,0,0,-1]\n", 0),
        ("UTC", "", "%OS", "[1900,0,0,0,0,0,-1]\n", 0),
        ("UTC", "", "%Y%M%S%y", "[1900,0,0,0,0,0,-1]\n", 0),
        ("UTC", "  ", "%Y%n", "[1900,0,0,0,0,0,-1]\n", 0),
        ("UTC", "  ", "%Y %n", "", 5),
        ("UTC", "  ", " %Y", "[1900,0,0,0,0,0,-1]\n", 0),
        ("UTC", "10:", "%H:%M", "[1900,0,0,10,0,0,-1]\n", 0),
        ("UTC", "10:", "%R", "[1900,0,0,10,0,0,-1]\n", 0),
        ("UTC", "10: ", "%R", "[1900,0,0,10,0,0,-1,\" \"]\n", 0),
        ("UTC", "10:30:", "%T", "[1900,0,0,10,30,0,-1]\n", 0),
        ("UTC", "10:30:", "%X", "[1900,0,0,10,30,0,-1]\n", 0),
        ("UTC", "10:30: AM", "%r", "[1900,0,0,10,30,0,-1]\n", 0),
        ("UTC", "10::", "%T", "", 5),
        ("UTC", "10: :00", "%T", "", 5),
        ("UTC", "1015", "%H%M", "[1900,0,0,10,15,0,-1]\n", 0),
        ("UTC", "10", "%H%M", "[1900,0,0,10,0,0,-1]\n", 0),
        ("UTC", "01/05/", "%D", "[1900,0,5,0,0,0,4]\n", 0),
        ("UTC", "01/05/ ", "%D", "[1900,0,5,0,0,0,4,\" \"]\n", 0),
        ("UTC", "01/05/", "%x", "[1900,0,5,0,0,0,4]\n", 0),
        ("UTC", "20 01/05/", "%C %D", "[2000,0,5,0,0,0,4]\n", 0),
        ("UTC", "20 01/05/ 70", "%C %D %y", "[1970,0,5,0,0,0,4]\n", 0),
        ("UTC", "20 70", "%C%Y %y", "[2070,0,0,0,0,0,-1]\n", 0),
        ("UTC", "20 ", "%C%y", "[2000,0,0,0,0,0,-1,\" \"]\n", 0),
        ("UTC", "15", "%C%y%Y", "[1500,0,0,0,0,0,-1]\n", 0),
        ("UTC", " 09", "%Y%n%U", "[1900,0,0,0,0,0,-1]\n", 0),
        ("UTC", "x", "%Y", "", 5),
        ("UTC", " x", "%Yx", "", 5),
        ("UTC", " 5", "%Y%m", "", 5),
        ("UTC", "", "%C", "", 5),
        ("UTC", "", "%m", "", 5),
        ("UTC", "", "%H", "", 5),
        ("UTC", "", "%j", "", 5),
        ("UTC", "", "%U", "", 5),
        ("UTC", "", "%a", "", 5),
        ("UTC", "", "%Z", "", 5),
        ("UTC", "2015-03-", "%F", "", 5),
    ])
}

/// #5947: `%s` reads as `strtol` does: with no digits it reads 0 and consumes nothing, so what follows must still match; a month or day out of range before it is refused as it is read.
#[test]
fn test_5947_strptime_epoch_seconds_with_no_digits_read_zero() -> Result<()> {
    assert_rows(&[
        ("UTC", "", "%s", "[1970,0,1,0,0,0,0]\n", 0),
        ("UTC", " ", "%s", "[1970,0,1,0,0,0,0,\" \"]\n", 0),
        ("Australia/Sydney", "", "%s", "[1970,0,1,10,0,0,0]\n", 0),
        ("UTC", "+", "%s", "", 5),
        ("UTC", "-", "%s", "", 5),
        ("UTC", "x", "%s", "", 5),
        ("UTC", "2015", "%Y%s", "[1970,0,1,0,0,0,0]\n", 0),
        ("UTC", "2015 ", "%Y %s", "[1970,0,1,0,0,0,0]\n", 0),
        ("UTC", "2015 x", "%Y%sx", "", 5),
        ("UTC", "  12", "%s", "[1970,0,1,0,0,12,0]\n", 0),
        ("UTC", "69", "%d %s", "", 5),
        ("UTC", "13", "%m%s", "", 5),
        ("UTC", "13 ", "%m %s", "", 5),
        ("UTC", "00", "%d%s", "[1970,0,1,0,0,0,0]\n", 0),
        ("UTC", "32", "%e%s", "", 5),
        ("UTC", "13/05/15", "%D%s", "", 5),
        ("UTC", "2015-03-32", "%F%s", "", 5),
    ])
}

/// #5947: `%Y`, `%y` and `%C` (and the years inside `%D` and `%F`) take a sign, which uses one character of the field's width; a sign alone reads 0. A negative year places a week number with the libc's first-weekday arithmetic.
#[test]
fn test_5947_strptime_year_fields_take_a_sign() -> Result<()> {
    assert_rows(&[
        ("UTC", "+", "%Y", "[0,0,0,0,0,0,-1]\n", 0),
        ("UTC", "-", "%Y", "[0,0,0,0,0,0,-1]\n", 0),
        ("UTC", "+ 5", "%Y", "[0,0,0,0,0,0,-1,\" 5\"]\n", 0),
        ("UTC", "+5", "%Y", "[5,0,0,0,0,0,-1]\n", 0),
        ("UTC", "-5", "%Y", "[-5,0,0,0,0,0,-1]\n", 0),
        ("UTC", "+201", "%Y", "[201,0,0,0,0,0,-1]\n", 0),
        ("UTC", "-201", "%Y", "[-201,0,0,0,0,0,-1]\n", 0),
        ("UTC", "+2015", "%Y", "", 5),
        ("UTC", "+-5", "%Y", "", 5),
        ("UTC", "+", "%y", "[2000,0,0,0,0,0,-1]\n", 0),
        ("UTC", "+5", "%y", "[2005,0,0,0,0,0,-1]\n", 0),
        ("UTC", "-5", "%y", "[1995,0,0,0,0,0,-1]\n", 0),
        ("UTC", "+20", "%y", "", 5),
        ("UTC", "+", "%C", "[0,0,0,0,0,0,-1]\n", 0),
        ("UTC", "+1", "%C", "[100,0,0,0,0,0,-1]\n", 0),
        ("UTC", "-5", "%C", "[-500,0,0,0,0,0,-1]\n", 0),
        ("UTC", "+12", "%C", "", 5),
        ("UTC", "-1 15", "%C %y", "[-85,0,0,0,0,0,-1]\n", 0),
        ("UTC", "-5 -9", "%y %C", "[-805,0,0,0,0,0,-1]\n", 0),
        ("UTC", "-5 +1", "%y %C", "[195,0,0,0,0,0,-1]\n", 0),
        ("UTC", "-5-03-05", "%F", "[-5,2,5,0,0,0,63]\n", 0),
        ("UTC", "01/05/-5", "%D", "[1995,0,5,0,0,0,4]\n", 0),
        ("UTC", "01/05/+", "%D", "[2000,0,5,0,0,0,4]\n", 0),
        ("UTC", "+5", "%m", "", 5),
        ("UTC", "-5", "%M", "", 5),
        ("UTC", "-5 09", "%Y %U", "[-5,2,4,0,0,0,62]\n", 0),
        ("UTC", "-999 09", "%Y %U", "[-999,1,28,0,0,0,58]\n", 0),
        ("UTC", "-999 09", "%Y %V", "[-999,2,1,0,0,0,59]\n", 0),
        ("UTC", "-100 01", "%Y %W", "[-100,0,7,0,0,0,6]\n", 0),
        ("UTC", "-5 00 4", "%Y %U %w", "[-5,0,4,0,0,0,3]\n", 0),
        ("UTC", "-999 366", "%Y %j", "[-998,0,1,0,0,0,0]\n", 0),
        ("UTC", "-991 01", "%Y %V", "[-991,0,1,0,0,0,0]\n", 0),
        ("UTC", "-991 53", "%Y %V", "[-991,11,31,0,0,0,364]\n", 0),
        ("UTC", "-985 09", "%Y %V", "[-985,1,26,0,0,0,56]\n", 0),
    ])
}
