//! #5754: `strptime` reads `%U`, `%W` and `%V` and, once a year is read, places the date by them as
//! the pinned libc does: `%U` counts weeks from the year's first Sunday, `%W` from its first Monday
//! and `%V` from the Monday of ISO week 1; the weekday read (`%w`, `%u`, `%a`) picks the day within
//! the week. `%w` and `%u` refuse a weekday out of range. Every row is what jq 1.7.1
//! (`/usr/bin/jq`, macOS) prints, through both routes; rows where that jq aborts on its own
//! `set_tm_yday` assertion (a day read that disagrees with the week) have no answer to match and
//! are left out.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

/// #5754: `%U` counts from the year's first Sunday, `%W` from its first Monday and `%V` from the Monday of ISO week 1, in either order with the year, from `%y` and `%C` as well, and through `%O`/`%E`.
#[test]
fn test_5754_strptime_week_number_with_a_year_sets_the_date() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            "null",
            r#""2015 09" | strptime("%Y %U")"#,
            "[2015,2,1,0,0,0,0,59]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2015 09" | strptime("%Y %W")"#,
            "[2015,2,2,0,0,0,1,60]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2015 09" | strptime("%Y %V")"#,
            "[2015,1,23,0,0,0,1,53]\n",
            "",
            0,
        ),
        (
            "null",
            r#""09 2015" | strptime("%U %Y")"#,
            "[2015,2,1,0,0,0,0,59]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2015 9" | strptime("%Y %U")"#,
            "[2015,2,1,0,0,0,0,59]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2017 01" | strptime("%Y %U")"#,
            "[2017,0,1,0,0,0,0,0]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2018 01" | strptime("%Y %W")"#,
            "[2018,0,1,0,0,0,1,0]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2016 01" | strptime("%Y %V")"#,
            "[2016,0,4,0,0,0,1,3]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2018 01" | strptime("%Y %V")"#,
            "[2018,0,1,0,0,0,1,0]\n",
            "",
            0,
        ),
        (
            "null",
            r#""15 09" | strptime("%y %U")"#,
            "[2015,2,1,0,0,0,0,59]\n",
            "",
            0,
        ),
        (
            "null",
            r#""20 09" | strptime("%C %U")"#,
            "[2000,1,27,0,0,0,0,57]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2015 09" | strptime("%Y %OU")"#,
            "[2015,2,1,0,0,0,0,59]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2015 09" | strptime("%Y %EW")"#,
            "[2015,2,2,0,0,0,1,60]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2015 09" | strptime("%Y %OV")"#,
            "[2015,1,23,0,0,0,1,53]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2015 09 10" | strptime("%Y %U %W")"#,
            "[2015,2,9,0,0,0,1,67]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2015 10 09" | strptime("%Y %W %U")"#,
            "[2015,2,1,0,0,0,0,59]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2015 09 rest" | strptime("%Y %U")"#,
            "[2015,2,1,0,0,0,0,59,\" rest\"]\n",
            "",
            0,
        ),
    ])
}

/// #5754: `%w`, `%u` and `%a` pick the day within the week, read before or after it; `%U` weeks start on Sunday, `%W` and `%V` weeks on Monday, so Sunday ends them.
#[test]
fn test_5754_strptime_week_number_takes_the_weekday_read() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            "null",
            r#""2015 09 3" | strptime("%Y %U %w")"#,
            "[2015,2,4,0,0,0,3,62]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2015 09 3" | strptime("%Y %W %w")"#,
            "[2015,2,4,0,0,0,3,62]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2015 09 0" | strptime("%Y %W %w")"#,
            "[2015,2,8,0,0,0,0,66]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2015 09 0" | strptime("%Y %V %w")"#,
            "[2015,2,1,0,0,0,0,59]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2015 09 7" | strptime("%Y %V %u")"#,
            "[2015,2,1,0,0,0,0,59]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2015 09 Wed" | strptime("%Y %U %a")"#,
            "[2015,2,4,0,0,0,3,62]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2015 09 Monday" | strptime("%Y %U %A")"#,
            "[2015,2,2,0,0,0,1,60]\n",
            "",
            0,
        ),
        (
            "null",
            r#""3 2015 09" | strptime("%w %Y %U")"#,
            "[2015,2,4,0,0,0,3,62]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2016 01 0" | strptime("%Y %V %w")"#,
            "[2016,0,10,0,0,0,0,9]\n",
            "",
            0,
        ),
    ])
}

/// #5754: Week 0 holds the days before the first counted week: a day of it in the year is read, one before the year (or with no weekday read) is no match, and `%V` has no week 0. Week 53 rolls into the next year.
#[test]
fn test_5754_strptime_week_zero_and_week_53() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            "null",
            r#""2015 00 4" | strptime("%Y %U %w")"#,
            "[2015,0,1,0,0,0,4,0]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2015 00 6" | strptime("%Y %U %w")"#,
            "[2015,0,3,0,0,0,6,2]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2015 00 0" | strptime("%Y %W %w")"#,
            "[2015,0,4,0,0,0,0,3]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2018 00 4" | strptime("%Y %U %w")"#,
            "[2018,0,4,0,0,0,4,3]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2015 00" | strptime("%Y %U")"#,
            "",
            "date \"2015 00\" does not match format \"%Y %U\"",
            5,
        ),
        (
            "null",
            r#""2015 00 1" | strptime("%Y %W %w")"#,
            "",
            "date \"2015 00 1\" does not match format \"%Y %W %w\"",
            5,
        ),
        (
            "null",
            r#""2016 00 4" | strptime("%Y %U %w")"#,
            "",
            "date \"2016 00 4\" does not match format \"%Y %U %w\"",
            5,
        ),
        (
            "null",
            r#""2017 00 6" | strptime("%Y %U %w")"#,
            "",
            "date \"2017 00 6\" does not match format \"%Y %U %w\"",
            5,
        ),
        (
            "null",
            r#""2016 00 0" | strptime("%Y %V %w")"#,
            "",
            "date \"2016 00 0\" does not match format \"%Y %V %w\"",
            5,
        ),
        (
            "null",
            r#""2015 01" | strptime("%Y %V")"#,
            "",
            "date \"2015 01\" does not match format \"%Y %V\"",
            5,
        ),
        (
            "null",
            r#""2020 01" | strptime("%Y %V")"#,
            "",
            "date \"2020 01\" does not match format \"%Y %V\"",
            5,
        ),
        (
            "null",
            r#""2015 53" | strptime("%Y %U")"#,
            "[2016,0,3,0,0,0,0,2]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2015 53 6" | strptime("%Y %U %w")"#,
            "[2016,0,9,0,0,0,6,8]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2016 53 6" | strptime("%Y %U %w")"#,
            "[2017,0,7,0,0,0,6,6]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2015 53" | strptime("%Y %V")"#,
            "[2015,11,28,0,0,0,1,361]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2016 53" | strptime("%Y %V")"#,
            "[2017,0,2,0,0,0,1,1]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2018 53" | strptime("%Y %V")"#,
            "[2018,11,31,0,0,0,1,364]\n",
            "",
            0,
        ),
    ])
}

/// #5754: A week is one or two digits, 0 to 53 (`%V` 1 to 53), and with no year read it sets nothing.
#[test]
fn test_5754_strptime_week_number_range_and_digits() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            "null",
            r#""2015 54" | strptime("%Y %U")"#,
            "",
            "date \"2015 54\" does not match format \"%Y %U\"",
            5,
        ),
        (
            "null",
            r#""2015 60" | strptime("%Y %W")"#,
            "",
            "date \"2015 60\" does not match format \"%Y %W\"",
            5,
        ),
        (
            "null",
            r#""2015 54" | strptime("%Y %V")"#,
            "",
            "date \"2015 54\" does not match format \"%Y %V\"",
            5,
        ),
        (
            "null",
            r#""54" | strptime("%U")"#,
            "",
            "date \"54\" does not match format \"%U\"",
            5,
        ),
        (
            "null",
            r#""00" | strptime("%V")"#,
            "",
            "date \"00\" does not match format \"%V\"",
            5,
        ),
        (
            "null",
            r#""09 03 05" | strptime("%U %m %d")"#,
            "[1900,2,5,0,0,0,1,63]\n",
            "",
            0,
        ),
        (
            "null",
            r#""00 03 05" | strptime("%W %m %d")"#,
            "[1900,2,5,0,0,0,1,63]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2015 009" | strptime("%Y %U")"#,
            "",
            "date \"2015 009\" does not match format \"%Y %U\"",
            5,
        ),
        (
            "null",
            r#""2015 xx" | strptime("%Y %U")"#,
            "",
            "date \"2015 xx\" does not match format \"%Y %U\"",
            5,
        ),
        (
            "null",
            r#""2015 +9" | strptime("%Y %U")"#,
            "",
            "date \"2015 +9\" does not match format \"%Y %U\"",
            5,
        ),
    ])
}

/// #5754: A month read is kept and the day counted from its start, past 31 or below 1; a day read is kept beside the month the week falls in; a month and day together, `%j` and `%s` win over the week.
#[test]
fn test_5754_strptime_week_number_next_to_a_month_or_day() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            "null",
            r#""2015 01 09" | strptime("%Y %m %U")"#,
            "[2015,0,60,0,0,0,0,59]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2015 12 09" | strptime("%Y %m %U")"#,
            "[2015,11,-274,0,0,0,0,59]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2015 Jan 09" | strptime("%Y %b %U")"#,
            "[2015,0,60,0,0,0,0,59]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2015 01 53" | strptime("%Y %m %U")"#,
            "[2015,0,368,0,0,0,0,367]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2015 01 09" | strptime("%Y %d %U")"#,
            "[2015,2,1,0,0,0,0,59]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2015 03 53" | strptime("%Y %d %U")"#,
            "[2016,0,3,0,0,0,0,2]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2015 09 01" | strptime("%Y %U %e")"#,
            "[2015,2,1,0,0,0,0,59]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2015 03 05 09" | strptime("%Y %m %d %U")"#,
            "[2015,2,5,0,0,0,4,63]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2015-01-01 09" | strptime("%F %U")"#,
            "[2015,0,1,0,0,0,4,0]\n",
            "",
            0,
        ),
        (
            "null",
            r#""01/05/15 09" | strptime("%D %U")"#,
            "[2015,0,5,0,0,0,1,4]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2015 100 09" | strptime("%Y %j %U")"#,
            "[2015,3,10,0,0,0,5,99]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2015 09 100" | strptime("%Y %U %j")"#,
            "[2015,3,10,0,0,0,5,99]\n",
            "",
            0,
        ),
    ])
}

/// #5754: `%w` is 0 to 6 and `%u` 1 to 7, with or without a week number.
#[test]
fn test_5754_strptime_weekday_range() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            "null",
            r#""2015 7" | strptime("%Y %w")"#,
            "",
            "date \"2015 7\" does not match format \"%Y %w\"",
            5,
        ),
        (
            "null",
            r#""2015 0" | strptime("%Y %u")"#,
            "",
            "date \"2015 0\" does not match format \"%Y %u\"",
            5,
        ),
        (
            "null",
            r#""2015 8" | strptime("%Y %u")"#,
            "",
            "date \"2015 8\" does not match format \"%Y %u\"",
            5,
        ),
        (
            "null",
            r#""2015 09 7" | strptime("%Y %U %w")"#,
            "",
            "date \"2015 09 7\" does not match format \"%Y %U %w\"",
            5,
        ),
        (
            "null",
            r#""2015 09 0" | strptime("%Y %U %u")"#,
            "",
            "date \"2015 09 0\" does not match format \"%Y %U %u\"",
            5,
        ),
        (
            "null",
            r#""2015 6" | strptime("%Y %w")"#,
            "[2015,0,0,0,0,0,3,-1]\n",
            "",
            0,
        ),
        (
            "null",
            r#""2015 7" | strptime("%Y %u")"#,
            "[2015,0,0,0,0,0,3,-1]\n",
            "",
            0,
        ),
    ])
}
