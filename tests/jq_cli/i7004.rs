//! #7004: `strftime`, `strflocaltime`, `todate` and `todateiso8601` format a number through the array
//! `gmtime`/`localtime` return, as jq does, whose seconds slot is `tm_sec + (t - floor(t))` read back
//! as an integer. Just below zero that sum rounds to `1.0` (`-1e-17 - (-1)`), so jq prints second
//! `01`, where the truncated timestamp's own second is `00`. Every row was captured from jq 1.7.1
//! on macOS under the zone it names.

use std::process::Command;

use anyhow::Result;

use crate::{spawn_with_signal_retry, succinctly_bin};

/// `(TZ, input, filter, stdout)`.
const ROWS: &[(&str, &str, &str, &str)] = &[
    ("UTC", "-1e-17", "todate", "\"1970-01-01T00:00:01Z\"\n"),
    (
        "UTC",
        "-1e-17",
        "todateiso8601",
        "\"1970-01-01T00:00:01Z\"\n",
    ),
    (
        "UTC",
        "-1e-17",
        "strftime(\"%S %T %c\")",
        "\"01 00:00:01 Thu Jan  1 00:00:01 1970\"\n",
    ),
    ("UTC", "-1e-300", "todate", "\"1970-01-01T00:00:01Z\"\n"),
    (
        "UTC",
        "-1e-300",
        "todateiso8601",
        "\"1970-01-01T00:00:01Z\"\n",
    ),
    (
        "UTC",
        "-1e-300",
        "strftime(\"%S %T %c\")",
        "\"01 00:00:01 Thu Jan  1 00:00:01 1970\"\n",
    ),
    ("UTC", "-1e-16", "todate", "\"1970-01-01T00:00:00Z\"\n"),
    (
        "UTC",
        "-1e-16",
        "todateiso8601",
        "\"1970-01-01T00:00:00Z\"\n",
    ),
    (
        "UTC",
        "-1e-16",
        "strftime(\"%S %T %c\")",
        "\"00 00:00:00 Thu Jan  1 00:00:00 1970\"\n",
    ),
    ("UTC", "-0.5", "todate", "\"1970-01-01T00:00:00Z\"\n"),
    ("UTC", "-0.5", "todateiso8601", "\"1970-01-01T00:00:00Z\"\n"),
    (
        "UTC",
        "-0.5",
        "strftime(\"%S %T %c\")",
        "\"00 00:00:00 Thu Jan  1 00:00:00 1970\"\n",
    ),
    (
        "UTC",
        "59.99999999999999",
        "todate",
        "\"1970-01-01T00:00:59Z\"\n",
    ),
    (
        "UTC",
        "59.99999999999999",
        "todateiso8601",
        "\"1970-01-01T00:00:59Z\"\n",
    ),
    (
        "UTC",
        "59.99999999999999",
        "strftime(\"%S %T %c\")",
        "\"59 00:00:59 Thu Jan  1 00:00:59 1970\"\n",
    ),
    ("UTC", "1e-17", "todate", "\"1970-01-01T00:00:00Z\"\n"),
    (
        "UTC",
        "1e-17",
        "todateiso8601",
        "\"1970-01-01T00:00:00Z\"\n",
    ),
    (
        "UTC",
        "1e-17",
        "strftime(\"%S %T %c\")",
        "\"00 00:00:00 Thu Jan  1 00:00:00 1970\"\n",
    ),
    ("UTC", "-1e-17", "strftime(\"%s\")", "\"1\"\n"),
    ("UTC", "-1e-17", "todate | fromdate", "1\n"),
    (
        "UTC",
        "-1e-17",
        "strftime(\"%S\") == (gmtime | strftime(\"%S\"))",
        "true\n",
    ),
    ("UTC", "nan", "strftime(\"%S\")", "\"00\"\n"),
    (
        "UTC",
        "-1e-17",
        "strflocaltime(\"%S %T\")",
        "\"01 00:00:01\"\n",
    ),
    (
        "UTC",
        "-0.5",
        "strflocaltime(\"%S %T\")",
        "\"00 00:00:00\"\n",
    ),
    (
        "UTC",
        "1e-17",
        "strflocaltime(\"%S %T\")",
        "\"00 00:00:00\"\n",
    ),
    (
        "EST5EDT",
        "-1e-17",
        "strflocaltime(\"%S %T\")",
        "\"01 19:00:01\"\n",
    ),
    (
        "EST5EDT",
        "-0.5",
        "strflocaltime(\"%S %T\")",
        "\"00 19:00:00\"\n",
    ),
    (
        "EST5EDT",
        "1e-17",
        "strflocaltime(\"%S %T\")",
        "\"00 19:00:00\"\n",
    ),
    (
        "Asia/Kolkata",
        "-1e-17",
        "strflocaltime(\"%S %T\")",
        "\"01 05:30:01\"\n",
    ),
    (
        "Asia/Kolkata",
        "-0.5",
        "strflocaltime(\"%S %T\")",
        "\"00 05:30:00\"\n",
    ),
    (
        "Asia/Kolkata",
        "1e-17",
        "strflocaltime(\"%S %T\")",
        "\"00 05:30:00\"\n",
    ),
    (
        "EST5EDT",
        "-1e-17",
        "strftime(\"%S %T\")",
        "\"01 00:00:01\"\n",
    ),
    (
        "Asia/Kolkata",
        "-1e-17",
        "strftime(\"%S %T\")",
        "\"01 00:00:01\"\n",
    ),
];

#[test]
fn a_number_is_formatted_through_its_broken_down_time_7004() -> Result<()> {
    for &(zone, input, filter, want) in ROWS {
        let (output, code) = spawn_with_signal_retry(
            || {
                let mut command = Command::new(succinctly_bin());
                command.env("TZ", zone).args([
                    "jq",
                    "-nc",
                    "--argjson",
                    "x",
                    input,
                    &format!("$x | {filter}"),
                ]);
                command
            },
            None,
        )?;
        let stdout = String::from_utf8(output.stdout)?;
        assert_eq!(
            (stdout.as_str(), code),
            (want, 0),
            "TZ={zone} {input} | {filter}"
        );
    }
    Ok(())
}
