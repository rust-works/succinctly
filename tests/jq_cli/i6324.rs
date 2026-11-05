//! #6324: jq hands `strptime` the date string and the format as C strings, so a NUL byte ends each:
//! `"2015-03-05T23:51:47Z\u0000junk" | fromdate` parses, and the error text of a failure shows the
//! string cut there. Every row was captured from jq 1.7.1 (`/usr/bin/jq -nc`).

use anyhow::Result;

use crate::run_jq_full;

fn assert_rows(rows: &[(&str, &str)]) -> Result<()> {
    for (filter, expected) in rows {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(
            (stdout.trim_end(), code),
            (*expected, 0),
            "{filter}: stderr {stderr}"
        );
    }
    Ok(())
}

#[test]
fn test_a_nul_ends_the_date_string_and_the_format_6324() -> Result<()> {
    assert_rows(&[
        (
            r#""2015-03-05T23:51:47Z\u0000junk" | fromdate"#,
            "1425599507",
        ),
        (
            r#""2015-03-05T23:51:47Z\u0000junk" | fromdateiso8601"#,
            "1425599507",
        ),
        (
            r#""2015-03-05T23:51:47Z\u0000" | strptime("%Y-%m-%dT%H:%M:%SZ") | mktime"#,
            "1425599507",
        ),
        (
            r#""2015-03-05" | strptime("%Y-%m-%d\u0000junk") | mktime"#,
            "1425513600",
        ),
        (
            r#""2015\u00002016" | strptime("%Y")"#,
            "[2015,0,0,0,0,0,3,-1]",
        ),
        (
            r#""2015-03-05T23:51:47Z" | strptime("%Y-%m-%dT%H:%M:%SZ\u0000x") | todate"#,
            r#""2015-03-05T23:51:47Z""#,
        ),
    ])
}

#[test]
fn test_the_error_text_shows_the_string_cut_at_the_nul_6324() -> Result<()> {
    assert_rows(&[
        (
            r#""\u0000" | try fromdate catch ."#,
            r#""date \"\" does not match format \"%Y-%m-%dT%H:%M:%SZ\"""#,
        ),
        (
            r#""\u0000" | try fromdateiso8601 catch ."#,
            r#""date \"\" does not match format \"%Y-%m-%dT%H:%M:%SZ\"""#,
        ),
        (
            r#""x" | try strptime("\u0000") catch ."#,
            r#""date \"x\" does not match format \"\"""#,
        ),
        (
            r#""abc\u0000def" | try strptime("%Y") catch ."#,
            r#""date \"abc\" does not match format \"%Y\"""#,
        ),
        (r#""x\u0000" | [strptime("%Y")?]"#, "[]"),
    ])
}
