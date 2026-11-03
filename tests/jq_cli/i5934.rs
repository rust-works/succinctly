//! #5934: `strptime` resolves `%C`, `%y` and `%Y` in the order the libc reads them, and takes `%n`
//! and `%t` for at least one white-space character. A `%C` after a `%Y` replaces the whole year with
//! the century (`"2015 20" | strptime("%Y %C")` is year 2000), a `%C` and a `%y` complete each other
//! whichever comes first and use each other up, and `%Y`, `%D` and `%F` clear a pending half. Every
//! expectation was captured from jq 1.7.1 (`/usr/bin/jq`, macOS). The rows avoid the century years
//! (1800, 1900, 2100), whose weekday jq's own routine gets wrong, and white-space-only input, which jq
//! reads as the zeroed time: both are separate differences.

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
fn century_and_two_digit_year_complete_each_other_in_the_libcs_order() -> Result<()> {
    check(&[
        ("2015 20", "%Y %C", Some("[2000,0,0,0,0,0,5,-1]")),
        ("201520", "%Y%C", Some("[2000,0,0,0,0,0,5,-1]")),
        ("1987 20", "%Y %C", Some("[2000,0,0,0,0,0,5,-1]")),
        ("2015 19 05", "%Y %C %y", Some("[1905,0,0,0,0,0,6,-1]")),
        ("2015 05 20", "%Y %y %C", Some("[2005,0,0,0,0,0,5,-1]")),
        ("20 85", "%C %y", Some("[2085,0,0,0,0,0,0,-1]")),
        ("85 20", "%y %C", Some("[2085,0,0,0,0,0,0,-1]")),
        ("20 2015 05", "%C %Y %y", Some("[2005,0,0,0,0,0,5,-1]")),
        ("20 1987 85", "%C %Y %y", Some("[1985,0,0,0,0,0,1,-1]")),
        ("05 2015 20", "%y %Y %C", Some("[2000,0,0,0,0,0,5,-1]")),
        ("20 05 20", "%C %y %C", Some("[2000,0,0,0,0,0,5,-1]")),
        ("20 05 06", "%C %y %y", Some("[2006,0,0,0,0,0,6,-1]")),
        ("05 20 06", "%y %C %y", Some("[2006,0,0,0,0,0,6,-1]")),
        ("2015 20", "%Y %C", Some("[2000,0,0,0,0,0,5,-1]")),
        ("20 03/05/15", "%C %D", Some("[2015,2,5,0,0,0,4,63]")),
        ("03/05/15 20", "%D %C", Some("[2000,2,5,0,0,0,0,64]")),
        ("2015-03-05 20", "%F %C", Some("[2000,2,5,0,0,0,0,64]")),
        ("20 2015-03-05", "%C %F", Some("[2015,2,5,0,0,0,4,63]")),
        (
            "2015 20 03 05",
            "%Y %C %m %d",
            Some("[2000,2,5,0,0,0,0,64]"),
        ),
    ])
}

#[test]
fn newline_and_tab_need_a_white_space_character_to_read() -> Result<()> {
    check(&[
        ("2015", "%Y%n", None),
        ("2015 ", "%Y%n", Some("[2015,0,0,0,0,0,3,-1]")),
        ("2015\n", "%Y%n", Some("[2015,0,0,0,0,0,3,-1]")),
        ("2015\t", "%Y%t", Some("[2015,0,0,0,0,0,3,-1]")),
        ("2015", "%Y%t", None),
        ("2015  ", "%Y %n", None),
        ("201503", "%Y%n%m", None),
        ("2015\n03", "%Y%n%m", Some("[2015,2,0,0,0,0,6,58]")),
        ("2015\t03", "%Y%t%m", Some("[2015,2,0,0,0,0,6,58]")),
        ("2015 03", "%Y%n%m", Some("[2015,2,0,0,0,0,6,58]")),
        ("2015", "%n%Y", None),
        ("2015 03", "%Y%n%m", Some("[2015,2,0,0,0,0,6,58]")),
        ("2015  \n 03", "%Y%n%m", Some("[2015,2,0,0,0,0,6,58]")),
    ])
}
