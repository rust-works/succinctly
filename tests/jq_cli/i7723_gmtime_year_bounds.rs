//! `gmtime` at the ends of the years a C `int` holds (the family of #7722 and #7723; found by the
//! pre-release differential, no issue of its own). jq's `tm2jv` adds 1900 to the `int` year of the
//! `struct tm`, so a year past 2147483647 prints wrapped, and `gmtime_r` refuses a year below
//! -2147481748. `succinctly jq` raises for the first and answers the second. Both differences are
//! kept and recorded in `docs/compliance/jq/limitations.md`; each row pins what `succinctly jq`
//! does, with jq's answer in its comment. The jq answers were captured from jq 1.7.1 on macOS
//! (`/usr/bin/jq`) and on Linux (glibc 2.43) and from jq 1.8.2 on macOS (`/opt/homebrew/bin/jq`),
//! which agree on every row here.

use anyhow::Result;

use crate::run_jq_null;

const REFUSAL: &str = "\"error converting number of seconds since epoch to datetime\"\n";

/// (program, stdout of `try (program) catch .`).
const ROWS: &[(&str, &str)] = &[
    // The last year that prints as itself: the same in jq.
    (
        "67767976233532000 | gmtime",
        "[2147483647,11,31,23,46,40,2,364]\n",
    ),
    // jq: `[-2147483648,0,1,0,0,0,3,0]` (year 2147483648, wrapped).
    ("67767976233532800 | gmtime", REFUSAL),
    // jq: `[-2147481749,0,1,0,0,0,3,0]` (year 2147485547, wrapped).
    ("67768036160140800 | gmtime", REFUSAL),
    // Past the last year a `struct tm` holds: the same error in jq.
    ("67768036191676800 | gmtime", REFUSAL),
    // The earliest year jq holds: the same in jq.
    (
        "-67768040609740800 | gmtime",
        "[-2147481748,0,1,0,0,0,4,0]\n",
    ),
    // jq: `error converting number of seconds since epoch to datetime` (exit 5).
    (
        "-67768040609750000 | gmtime",
        "[-2147481749,11,31,21,26,40,3,364]\n",
    ),
];

#[test]
fn gmtime_at_the_ends_of_the_c_int_years_7723() -> Result<()> {
    for &(program, want) in ROWS {
        let wrapped = format!("try ({program}) catch .");
        let (stdout, code) = run_jq_null(&wrapped, &["-c"])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{program}");
    }
    Ok(())
}
