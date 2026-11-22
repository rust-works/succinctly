//! #7722: jq's `f_mktime` reads a `timegm` result of `-2` as its "no `timegm` on this platform"
//! sentinel, so the instant 1969-12-31T23:59:58Z is `mktime not supported on this platform` (exit
//! 5), as `-1` is `invalid gmtime representation`. `fromdate` and `fromdateiso8601` are
//! `strptime(...) | mktime` and refuse it too; `strftime("%s")` prints `-2`. `succinctly jq`
//! answered `-2`. Every row was captured from jq 1.7.1 (`/usr/bin/jq`) and is the same in jq 1.8.2
//! (`/opt/homebrew/bin/jq`).

use std::process::Command;

use anyhow::Result;

use crate::{spawn_with_signal_retry, succinctly_bin};

/// (program under `-nc`, stdout).
const ROWS: &[(&str, &str)] = &[
    (
        "try ([1969,11,31,23,59,58,0,0] | mktime) catch .",
        "\"mktime not supported on this platform\"\n",
    ),
    (
        "try ([1970,0,1,0,0,-2,0,0] | mktime) catch .",
        "\"mktime not supported on this platform\"\n",
    ),
    (
        "try (\"1969-12-31T23:59:58Z\" | fromdate) catch .",
        "\"mktime not supported on this platform\"\n",
    ),
    (
        "try (\"1969-12-31T23:59:58Z\" | fromdateiso8601) catch .",
        "\"mktime not supported on this platform\"\n",
    ),
    (
        "try (\"1969-12-31T23:59:58Z\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\") | mktime) catch .",
        "\"mktime not supported on this platform\"\n",
    ),
    (
        "[(-2, -1, -3) | try (gmtime | mktime) catch .]",
        "[\"mktime not supported on this platform\",\"invalid gmtime representation\",-3]\n",
    ),
    ("[[1969,11,31,23,59,58,0,0] | mktime?]", "[]\n"),
    ("[1969,11,31,23,59,58,0,0] | strftime(\"%s\")", "\"-2\"\n"),
];

fn run(zone: &str, program: &str) -> Result<(String, String, i32)> {
    let (output, code) = spawn_with_signal_retry(
        || {
            let mut command = Command::new(succinctly_bin());
            command.env("TZ", zone).args(["jq", "-nc", program]);
            command
        },
        None,
    )?;
    Ok((
        String::from_utf8(output.stdout)?,
        String::from_utf8(output.stderr)?,
        code,
    ))
}

#[test]
fn mktime_of_minus_two_is_jqs_platform_error_7722() -> Result<()> {
    for zone in ["UTC", "Asia/Tokyo"] {
        for &(program, want) in ROWS {
            let (stdout, stderr, code) = run(zone, program)?;
            assert_eq!(
                (stdout.as_str(), code),
                (want, 0),
                "TZ={zone} {program}: {stderr}"
            );
        }
    }
    let (stdout, stderr, code) = run("UTC", "[1969,11,31,23,59,58,0,0] | mktime")?;
    assert_eq!(
        (stdout.as_str(), stderr.as_str(), code),
        (
            "",
            "jq: error (at <unknown>): mktime not supported on this platform\n",
            5
        )
    );
    Ok(())
}
