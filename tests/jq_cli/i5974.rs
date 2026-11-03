//! #5974: `strptime`'s `%d` and `%e` read white space before the day as the pinned libc does. A
//! white-space character takes the place of the first digit, so `%d` reads `" 5"` and `"\t5"` and
//! refuses `"  5"` and `" 05"`, `%e` skips one white-space character first and then reads as `%d`
//! does, a day above 31 is refused where it is read, and `%D`, `%F`, `%c` and `%x` read their day the
//! same way (they stand for `%d`). Every expectation was captured from jq 1.7.1 (`/usr/bin/jq`,
//! macOS). Most rows compare the first six fields only: the weekday of January 1900 is the macOS
//! routine's (#5946); the rows that name a year compare the whole array.

use anyhow::Result;

use crate::run_jq_null;

/// `$i|strptime($f)|.[0:6]` as compact JSON, or `None` when jq rejects the text (exit 5).
fn strptime(input: &str, fmt: &str) -> Result<Option<String>> {
    let (out, code) = run_jq_null(
        "$i|strptime($f)|.[0:6]",
        &["-c", "--arg", "i", input, "--arg", "f", fmt],
    )?;
    Ok(match code {
        0 => Some(out.trim_end().to_string()),
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
fn a_day_reads_the_white_space_before_it_as_the_libc_does() -> Result<()> {
    check(&[
        ("%d", " 5", Some("[1900,0,5,0,0,0]")),
        ("%d", "\t5", Some("[1900,0,5,0,0,0]")),
        ("%d", "\n5", Some("[1900,0,5,0,0,0]")),
        ("%d", " 0", Some("[1900,0,0,0,0,0]")),
        ("%d", "5", Some("[1900,0,5,0,0,0]")),
        ("%d", "05", Some("[1900,0,5,0,0,0]")),
        ("%d", "  5", None),
        ("%d", " 05", None),
        ("%d", "50", None),
        ("%d", "32", None),
        ("%d", "5 ", Some("[1900,0,5,0,0,0]")),
        ("%e", " 5", Some("[1900,0,5,0,0,0]")),
        ("%e", "  5", Some("[1900,0,5,0,0,0]")),
        ("%e", " 05", Some("[1900,0,5,0,0,0]")),
        ("%e", "\t\t5", Some("[1900,0,5,0,0,0]")),
        ("%e", "\t 5", Some("[1900,0,5,0,0,0]")),
        ("%e", "5", Some("[1900,0,5,0,0,0]")),
        ("%e", "05", Some("[1900,0,5,0,0,0]")),
        ("%e", "   5", None),
        ("%e", "  05", None),
        ("%e", "32", None),
        ("%Ed", " 5", Some("[1900,0,5,0,0,0]")),
        ("%Od", " 5", Some("[1900,0,5,0,0,0]")),
        ("%Ee", "  5", Some("[1900,0,5,0,0,0]")),
        ("%Oe", "  5", Some("[1900,0,5,0,0,0]")),
        ("%Ed", " 05", None),
        ("%d%d", " 5 5", Some("[1900,0,5,0,0,0]")),
        ("%d%d", "5 5", Some("[1900,0,5,0,0,0]")),
        ("%d%d", " 55", None),
        ("%d%d", "500", None),
        ("%d %d", " 5 5", Some("[1900,0,5,0,0,0]")),
        ("%e%e", "  5  5", Some("[1900,0,5,0,0,0]")),
        ("%e %d", "  5 5", Some("[1900,0,5,0,0,0]")),
        ("%F", "2015-03- 5", Some("[2015,2,5,0,0,0]")),
        ("%F", "2015-03-\t5", Some("[2015,2,5,0,0,0]")),
        ("%F", "2015-03-  5", None),
        ("%F", "2015-03- 05", None),
        ("%D", "03/ 5/15", Some("[2015,2,5,0,0,0]")),
        ("%D", "03/\t5/15", Some("[2015,2,5,0,0,0]")),
        ("%D", "03/  5/15", None),
        ("%D", "03/ 05/15", None),
        ("%Y-%m-%d", "2015-03- 5", Some("[2015,2,5,0,0,0]")),
        ("%Y-%m-%d", "2015-03-  5", None),
        ("%Y-%m-%e", "2015-03-  5", Some("[2015,2,5,0,0,0]")),
        ("%Y-%m-%e", "2015-03-   5", None),
        (
            "%a %b %e %H:%M:%S %Y",
            "Thu Mar  5 10:11:12 2015",
            Some("[2015,2,5,10,11,12]"),
        ),
        (
            "%c",
            "Thu Mar  5 10:11:12 2015",
            Some("[2015,2,5,10,11,12]"),
        ),
    ])
}

/// Vertical tab, form feed and carriage return are white space to the libc as space, tab and newline
/// are (`is_c_space`).
#[test]
fn every_c_white_space_character_pads_a_day() -> Result<()> {
    check(&[
        ("%d", "\x0b5", Some("[1900,0,5,0,0,0]")),
        ("%d", "\x0c5", Some("[1900,0,5,0,0,0]")),
        ("%d", "\r5", Some("[1900,0,5,0,0,0]")),
        ("%e", "\x0b\x0c5", Some("[1900,0,5,0,0,0]")),
    ])
}

/// A padded day lands on the weekday and yearday of its date, not only in the first six fields.
#[test]
fn a_padded_day_has_its_weekday_and_yearday() -> Result<()> {
    for (fmt, input) in [
        ("%F", "2015-03- 5"),
        ("%Y-%m-%e", "2015-03-  5"),
        ("%D", "03/\t5/15"),
        ("%Y %m %e", "2015 03 \x0b5"),
    ] {
        let (out, code) = run_jq_null(
            "$i|strptime($f)",
            &["-c", "--arg", "i", input, "--arg", "f", fmt],
        )?;
        assert_eq!(
            (out.trim_end(), code),
            ("[2015,2,5,0,0,0,4,63]", 0),
            "{input:?} | strptime({fmt:?})"
        );
    }
    Ok(())
}
