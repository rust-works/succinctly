//! #5934: `strptime`'s `%j` beside a month or a day. With a year read, a month read next to `%j`
//! keeps the month and counts the day from the month's start, so the day can pass 31 or go below 1
//! and the year does not roll (`"2015 03 050" | strptime("%Y %m %j")` is March -9); the yearday
//! printed is the `%j` as read. A day with no month derives the month from the day of year, and a
//! month and a day together stay as read. Every expectation was captured from jq 1.7.1
//! (`/usr/bin/jq`, macOS). The rows jq aborts on (its `set_tm_yday` assertion, whenever a day read
//! disagrees with the day of year, and a month with no year) have no answer to match.

use anyhow::Result;

use crate::run_jq_null;

/// `$i|strptime($f)` as compact JSON, or `None` when jq rejects the text (exit 5).
fn strptime(input: &str, fmt: &str) -> Result<Option<String>> {
    let (out, code) = run_jq_null(
        "$i|strptime($f)",
        &["-c", "--arg", "i", input, "--arg", "f", fmt],
    )?;
    Ok(match code {
        0 => Some(out.trim_end().to_string()),
        5 => None,
        other => panic!("{input:?} | strptime({fmt:?}) exited {other}"),
    })
}

fn check(rows: &[(&str, &str, Option<&str>)]) -> Result<()> {
    for &(input, fmt, want) in rows {
        assert_eq!(
            strptime(input, fmt)?.as_deref(),
            want,
            "{input:?} | strptime({fmt:?})"
        );
    }
    Ok(())
}

#[test]
fn a_month_read_beside_the_day_of_year_counts_the_day_from_the_months_start_5934() -> Result<()> {
    check(&[
        ("2015 03 050", "%Y %m %j", Some("[2015,2,-9,0,0,0,4,49]")),
        ("2015 050 03", "%Y %j %m", Some("[2015,2,-9,0,0,0,4,49]")),
        ("050 2015 03", "%j %Y %m", Some("[2015,2,-9,0,0,0,4,49]")),
        ("2015 Mar 050", "%Y %b %j", Some("[2015,2,-9,0,0,0,4,49]")),
        ("15 03 050", "%y %m %j", Some("[2015,2,-9,0,0,0,4,49]")),
        ("2015 02 001", "%Y %m %j", Some("[2015,1,-30,0,0,0,4,0]")),
        ("2015 02 059", "%Y %m %j", Some("[2015,1,28,0,0,0,6,58]")),
        ("2015 02 060", "%Y %m %j", Some("[2015,1,29,0,0,0,0,59]")),
        ("2016 03 060", "%Y %m %j", Some("[2016,2,0,0,0,0,1,59]")),
    ])
}

#[test]
fn the_day_of_year_beside_a_month_does_not_roll_into_the_next_year_5934() -> Result<()> {
    check(&[
        ("2015 02 366", "%Y %m %j", Some("[2015,1,335,0,0,0,5,365]")),
        ("2015 12 366", "%Y %m %j", Some("[2015,11,32,0,0,0,5,365]")),
        ("2016 02 366", "%Y %m %j", Some("[2016,1,335,0,0,0,6,365]")),
        ("2016 12 366", "%Y %m %j", Some("[2016,11,31,0,0,0,6,365]")),
        // With no month read it still rolls (#5383).
        ("2015 366", "%Y %j", Some("[2016,0,1,0,0,0,5,0]")),
    ])
}

#[test]
fn a_day_read_that_agrees_with_the_day_of_year_takes_its_month_from_it_5934() -> Result<()> {
    check(&[
        ("2015 19 050", "%Y %d %j", Some("[2015,1,19,0,0,0,4,49]")),
        ("2016 19 050", "%Y %d %j", Some("[2016,1,19,0,0,0,5,49]")),
        (
            "2015 02 19 050",
            "%Y %m %d %j",
            Some("[2015,1,19,0,0,0,4,49]"),
        ),
        (
            "2015 05 05 125",
            "%Y %m %d %j",
            Some("[2015,4,5,0,0,0,2,124]"),
        ),
        ("2015-02-19 050", "%F %j", Some("[2015,1,19,0,0,0,4,49]")),
        ("02/19/15 050", "%D %j", Some("[2015,1,19,0,0,0,4,49]")),
    ])
}

#[test]
fn the_year_rolled_by_the_day_of_year_and_other_year_spellings_5934() -> Result<()> {
    check(&[
        // A 366th day in a common year rolls the year even beside a day that agrees with the roll.
        ("2015 01 366", "%Y %d %j", Some("[2016,0,1,0,0,0,5,0]")),
        (
            "20 15 03 050",
            "%C %y %m %j",
            Some("[2015,2,-9,0,0,0,4,49]"),
        ),
        (
            "20 15 19 050",
            "%C %y %d %j",
            Some("[2015,1,19,0,0,0,4,49]"),
        ),
    ])
}

#[test]
fn a_month_out_of_range_beside_the_day_of_year_is_no_match_5934() -> Result<()> {
    check(&[
        ("2015 13 050", "%Y %m %j", None),
        ("2015 00 050", "%Y %m %j", None),
    ])
}
