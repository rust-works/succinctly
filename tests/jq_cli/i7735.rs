//! #7735: `nan | gmtime` and the date builtins that start from it answer the same on every platform.
//!
//! jq converts the seconds with `time_t secs = fsecs;`, undefined behaviour in C for NaN (unchanged
//! between 1.7.1 and 1.8.2): on x86_64 it gives `INT64_MIN` and jq raises `error converting number
//! of seconds since epoch to datetime` (jq 1.7.1 on x86_64 Linux aborts on `strflocaltime`); on arm64
//! it gives 0 and `nan | gmtime` is `[1970,0,1,0,0,null,4,0]`. `succinctly jq` gives the arm64 value
//! everywhere, and the builtins that format or convert it refuse its NaN field as jq 1.8.2 does
//! (#7532). A recorded divergence (limitations.md), pinned here so that a change does not make the
//! answer depend on the CPU.
//!
//! Every expectation is `succinctly jq`'s on every platform, and jq 1.8.2's on arm64 (captured from
//! `/opt/homebrew/bin/jq` on an Apple M5 Max under `TZ=UTC`); the `gmtime`/`localtime` rows are
//! jq 1.7.1's on arm64 too, and the `infinite` rows are jq 1.7.1's on x86_64 Linux as well.

use std::process::Command;

use anyhow::Result;

use crate::succinctly_bin;

/// `(program, stdout, stderr, exit status)` under `TZ=UTC`.
const ROWS: &[(&str, &str, &str, i32)] = &[
    ("nan | gmtime", "[1970,0,1,0,0,null,4,0]\n", "", 0),
    ("nan | localtime", "[1970,0,1,0,0,null,4,0]\n", "", 0),
    ("(-nan) | gmtime", "[1970,0,1,0,0,null,4,0]\n", "", 0),
    (
        "[nan | gmtime | .[] | isnan]",
        "[false,false,false,false,false,true,false,false]\n",
        "",
        0,
    ),
    (
        "nan | todate",
        "",
        "jq: error (at <unknown>): strftime/1 requires parsed datetime inputs\n",
        5,
    ),
    (
        "nan | todateiso8601",
        "",
        "jq: error (at <unknown>): strftime/1 requires parsed datetime inputs\n",
        5,
    ),
    (
        "nan | strftime(\"%Y\")",
        "",
        "jq: error (at <unknown>): strftime/1 requires parsed datetime inputs\n",
        5,
    ),
    (
        "nan | strflocaltime(\"%Y\")",
        "",
        "jq: error (at <unknown>): strflocaltime/1 requires parsed datetime inputs\n",
        5,
    ),
    (
        "nan | gmtime | mktime",
        "",
        "jq: error (at <unknown>): mktime requires parsed datetime inputs\n",
        5,
    ),
    (
        "infinite | gmtime",
        "",
        "jq: error (at <unknown>): error converting number of seconds since epoch to datetime\n",
        5,
    ),
    (
        "(-infinite) | gmtime",
        "",
        "jq: error (at <unknown>): error converting number of seconds since epoch to datetime\n",
        5,
    ),
];

#[test]
fn nan_date_builtins_answer_the_same_on_every_platform_7735() -> Result<()> {
    for &(program, stdout, stderr, status) in ROWS {
        let output = Command::new(succinctly_bin())
            .args(["jq", "-n", "-c", program])
            .env("TZ", "UTC")
            .output()?;
        assert_eq!(
            (
                String::from_utf8(output.stdout)?.as_str(),
                String::from_utf8(output.stderr)?.as_str(),
                output.status.code()
            ),
            (stdout, stderr, Some(status)),
            "{program}"
        );
    }
    Ok(())
}
