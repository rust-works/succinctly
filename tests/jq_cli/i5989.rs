//! #5989: `strptime` reads what the pinned libc allows between a `%` and its specifier: one `E` or
//! `O`, a `+` flag at the start or straight after the modifier, and a decimal width (`%0d`, `%+5Y`,
//! `%4s`, `%3m`). The width is the most digits a numeric field takes (a sign counts in a year's);
//! `%w`, `%u`, `%s`, the names, `%z` and the composite `%D %F %R %T` ignore it, `%c %x %X %r` stand
//! for their layouts under any prefix, and `%Z` with a width reads exactly that many capitals. A
//! padded `%d`/`%e` is one digit whatever the width. Every expectation was captured from jq 1.7.1
//! (`/usr/bin/jq`, macOS, `LC_ALL=C TZ=UTC`) by running the same text through its `strptime`; the
//! first six fields are compared, since the weekday of a century year in January is the macOS
//! routine's (#5946).

use std::process::Command;

use anyhow::Result;

use crate::{spawn_with_signal_retry, succinctly_bin};

/// `$i|strptime($f)|.[0:6]` under `TZ=UTC` and `LC_ALL=C` (the zone `%s` and `GMT` convert into, and
/// the locale whose names are read) as compact JSON, or `None` when jq rejects the text (exit 5).
fn strptime(input: &str, fmt: &str) -> Result<Option<String>> {
    let (output, code) = spawn_with_signal_retry(
        || {
            let mut command = Command::new(succinctly_bin());
            command.env("TZ", "UTC").env("LC_ALL", "C").args([
                "jq",
                "-nc",
                "--arg",
                "i",
                input,
                "--arg",
                "f",
                fmt,
                "$i|strptime($f)|.[0:6]",
            ]);
            command
        },
        None,
    )?;
    Ok(match code {
        0 => Some(String::from_utf8(output.stdout)?.trim_end().to_string()),
        5 => None,
        other => panic!("{input:?} | strptime({fmt:?}) exited {other}"),
    })
}

fn check(rows: &[(&str, &str, Option<&str>)]) -> Result<()> {
    for &(fmt, input, want) in rows {
        assert_eq!(
            strptime(input, fmt)?.as_deref(),
            want,
            "{input:?} | strptime({fmt:?})"
        );
    }
    Ok(())
}

#[test]
fn the_issue_table_reads_as_the_libc_does() -> Result<()> {
    check(&[
        ("%0d", "03", Some("[1900,0,3,0,0,0]")),
        ("%0m", "03", Some("[1900,2,0,0,0,0]")),
        ("%0Y", "2015", Some("[2015,0,0,0,0,0]")),
        ("%00Y", "2015", Some("[2015,0,0,0,0,0]")),
        ("%0EY", "2015", Some("[2015,0,0,0,0,0]")),
        ("%E0Y", "2015", Some("[2015,0,0,0,0,0]")),
        ("%+Y", "2015", Some("[2015,0,0,0,0,0]")),
        ("%2Y", "20", Some("[20,0,0,0,0,0]")),
        ("%2Y", "2015", None),
        ("%6Y", "201599", Some("[201599,0,0,0,0,0]")),
        ("%3m", "012", Some("[1900,11,0,0,0,0]")),
        ("%3m", "123", None),
        ("%+5Y", "-2015", Some("[-2015,0,0,0,0,0]")),
        ("%+Y", "+2015", None),
        ("%4s", "1425599621", Some("[2015,2,5,23,53,41]")),
        ("%2b", "Mar", Some("[1900,2,0,0,0,0]")),
        ("%3R", "10:30", Some("[1900,0,0,10,30,0]")),
        ("%_4Y", "2015", None),
        ("%-4Y", "2015", None),
        ("%0", "", None),
    ])
}

#[test]
fn the_prefix_grammar_is_one_modifier_a_leading_plus_and_digits() -> Result<()> {
    check(&[
        ("%4EY", "2015", Some("[2015,0,0,0,0,0]")),
        ("%E4Y", "2015", Some("[2015,0,0,0,0,0]")),
        ("%4OY", "2015", Some("[2015,0,0,0,0,0]")),
        ("%+4Y", "2015", Some("[2015,0,0,0,0,0]")),
        ("%4+Y", "2015", None),
        ("%04+Y", "2015", None),
        ("%+04Y", "2015", Some("[2015,0,0,0,0,0]")),
        ("%0+Y", "2015", None),
        ("%++Y", "2015", None),
        ("%EEY", "2015", None),
        ("%EOY", "2015", None),
        ("%E+Y", "2015", Some("[2015,0,0,0,0,0]")),
        ("%+E+Y", "2015", Some("[2015,0,0,0,0,0]")),
        ("%E++Y", "2015", None),
        ("%E0E", "2015", None),
        ("%4E5Y", "20150", Some("[20150,0,0,0,0,0]")),
        ("%5E4Y", "20150", None),
        ("%4E0Y", "20150", None),
        ("%5E0Y", "20150", Some("[20150,0,0,0,0,0]")),
        ("%5EY", "20150", Some("[20150,0,0,0,0,0]")),
        ("%E5Y", "20150", Some("[20150,0,0,0,0,0]")),
        ("%04Y", "20150", None),
        ("%4Y", "20150", None),
        ("%5Y", "20150", Some("[20150,0,0,0,0,0]")),
        ("%40Y", "20150", Some("[20150,0,0,0,0,0]")),
        ("%1Y", "20", None),
        ("%1Y", "2", Some("[2,0,0,0,0,0]")),
        ("%E%", "%", None),
        ("%E", "", None),
        ("%4%", "%", Some("[1900,0,0,0,0,0]")),
        ("%4%", "2015", None),
        ("%5n", "3", None),
        ("%5n%d", " 3", Some("[1900,0,3,0,0,0]")),
    ])
}

#[test]
fn a_width_is_the_most_digits_a_numeric_field_takes() -> Result<()> {
    check(&[
        ("%1d", "31", None),
        ("%1d", "3", Some("[1900,0,3,0,0,0]")),
        ("%3d", "003", Some("[1900,0,3,0,0,0]")),
        ("%3d", "005", Some("[1900,0,5,0,0,0]")),
        ("%2m", "03", Some("[1900,2,0,0,0,0]")),
        ("%1m", "12", None),
        ("%1m", "1", Some("[1900,0,0,0,0,0]")),
        ("%0H", "3", Some("[1900,0,0,3,0,0]")),
        ("%1H", "03", None),
        ("%1H", "0", Some("[1900,0,0,0,0,0]")),
        ("%2I", "12", Some("[1900,0,0,12,0,0]")),
        ("%0S", "5", Some("[1900,0,0,0,0,5]")),
        ("%3S", "005", Some("[1900,0,0,0,0,5]")),
        ("%3M", "030", Some("[1900,0,0,0,30,0]")),
        ("%4y", "2015", Some("[3915,0,0,0,0,0]")),
        ("%3C", "123", Some("[12300,0,0,0,0,0]")),
        ("%3C", "-12", Some("[-1200,0,0,0,0,0]")),
        ("%3U", "005", Some("[1900,0,0,0,0,0]")),
        ("%1U", "12", None),
        ("%1W", "5", Some("[1900,0,0,0,0,0]")),
        ("%1V", "1", Some("[1900,0,0,0,0,0]")),
        ("%2e", "3", Some("[1900,0,3,0,0,0]")),
        ("%3e", "003", Some("[1900,0,3,0,0,0]")),
        ("%3H", "005", Some("[1900,0,0,5,0,0]")),
        ("%3k", "005", Some("[1900,0,0,5,0,0]")),
        ("%3I", "005", Some("[1900,0,0,5,0,0]")),
        ("%3l", "005", Some("[1900,0,0,5,0,0]")),
    ])
}

#[test]
fn the_signed_year_counts_its_sign_in_the_width() -> Result<()> {
    check(&[
        ("%5Y", "-2015", Some("[-2015,0,0,0,0,0]")),
        ("%4Y", "-2015", None),
        ("%3Y", "-201", None),
        ("%5y", "-2015", Some("[-15,0,0,0,0,0]")),
        ("%3y", "-12", Some("[1988,0,0,0,0,0]")),
        ("%5C", "-1234", Some("[-123400,0,0,0,0,0]")),
        ("%+5Y", "+2015", Some("[2015,0,0,0,0,0]")),
        ("%5Y", "+2015", Some("[2015,0,0,0,0,0]")),
    ])
}

#[test]
fn a_padded_day_is_one_digit_whatever_the_width() -> Result<()> {
    check(&[
        ("%1d", " 1", Some("[1900,0,1,0,0,0]")),
        ("%1d", "  1", None),
        ("%1d", " 12", None),
        ("%3d", " 5", Some("[1900,0,5,0,0,0]")),
        ("%3d", " 05", None),
        ("%3d", "  5", None),
        ("%3d", " 123", None),
        ("%3d", " 12", None),
        ("%2d", " 5", Some("[1900,0,5,0,0,0]")),
        ("%1e", " 5", Some("[1900,0,5,0,0,0]")),
        ("%1e", "  5", Some("[1900,0,5,0,0,0]")),
        ("%2e", "  5", Some("[1900,0,5,0,0,0]")),
        ("%3e", "   5", None),
        ("%3e", "  05", None),
        ("%1e", "5", Some("[1900,0,5,0,0,0]")),
        ("%1e", "05", None),
        ("%0e", " 05", Some("[1900,0,5,0,0,0]")),
    ])
}

#[test]
fn w_u_s_the_names_and_the_composite_formats_ignore_a_width() -> Result<()> {
    check(&[
        ("%2w", "05", None),
        ("%3w", "5", Some("[1900,0,0,0,0,0]")),
        ("%2u", "05", None),
        ("%3u", "7", Some("[1900,0,0,0,0,0]")),
        ("%4s", "1425599621", Some("[2015,2,5,23,53,41]")),
        ("%+s", "1425599621", Some("[2015,2,5,23,53,41]")),
        ("%0s", "1425599621", Some("[2015,2,5,23,53,41]")),
        ("%2b", "March", Some("[1900,2,0,0,0,0]")),
        ("%9B", "March", Some("[1900,2,0,0,0,0]")),
        ("%2a", "Sun", Some("[1900,0,0,0,0,0]")),
        ("%2h", "Mar", Some("[1900,2,0,0,0,0]")),
        ("%3R", "10:30", Some("[1900,0,0,10,30,0]")),
        ("%3T", "10:30:15", Some("[1900,0,0,10,30,15]")),
        ("%5F", "2015-03-05", Some("[2015,2,5,0,0,0]")),
        ("%2D", "03/05/15", Some("[2015,2,5,0,0,0]")),
        ("%3p", "PM", Some("[1900,0,0,12,0,0]")),
        ("%9z", "+0100", Some("[1969,11,31,23,59,59]")),
        ("%2n", " ", Some("[1900,0,0,0,0,0]")),
        ("%2t", " ", Some("[1900,0,0,0,0,0]")),
    ])
}

#[test]
fn a_layout_takes_the_same_prefix() -> Result<()> {
    check(&[
        ("%0x", "03/05/15", Some("[2015,2,5,0,0,0]")),
        ("%5x", "03/05/15", Some("[2015,2,5,0,0,0]")),
        ("%+x", "03/05/15", Some("[2015,2,5,0,0,0]")),
        ("%E3x", "03/05/15", Some("[2015,2,5,0,0,0]")),
        ("%3Ex", "03/05/15", Some("[2015,2,5,0,0,0]")),
        ("%3X", "10:30:15", Some("[1900,0,0,10,30,15]")),
        ("%0X", "10:30:15", Some("[1900,0,0,10,30,15]")),
        ("%2r", "10:30:15 PM", Some("[1900,0,0,22,30,15]")),
        (
            "%5c",
            "Thu Mar  5 10:30:15 2015",
            Some("[2015,2,5,10,30,15]"),
        ),
        (
            "%Ec",
            "Thu Mar  5 10:30:15 2015",
            Some("[2015,2,5,10,30,15]"),
        ),
        ("%EEc", "Thu Mar  5 10:30:15 2015", None),
        ("%++x", "03/05/15", None),
        ("%4+x", "03/05/15", None),
        ("%%c", "%c", Some("[1900,0,0,0,0,0]")),
    ])
}

#[test]
fn a_zone_name_with_a_width_is_exactly_that_long() -> Result<()> {
    check(&[
        ("%Z", "GMT", Some("[1969,11,31,23,59,59]")),
        ("%0Z", "GMT", Some("[1969,11,31,23,59,59]")),
        ("%3Z", "GMT", Some("[1969,11,31,23,59,59]")),
        ("%2Z", "GMT", None),
        ("%4Z", "GMT", None),
        ("%1Z", "GMT", None),
        ("%5Z", "GMT", None),
        ("%3Z", "GMTA", None),
        ("%4Z", "GMTA", None),
        ("%3Z", "gmt", None),
        ("%3Z", "G1T", None),
        ("%+3Z", "GMT", Some("[1969,11,31,23,59,59]")),
        ("%E3Z", "GMT", Some("[1969,11,31,23,59,59]")),
        ("%Z", "GMTA", None),
    ])
}
