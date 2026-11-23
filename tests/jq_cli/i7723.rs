//! #7723: `mktime` near the C `int` limits. `timegm` stores the normalised fields back, so a year
//! that leaves C's `int` once 1900 is taken off is `invalid gmtime representation`, as in jq 1.8.2
//! (`/opt/homebrew/bin/jq`), and `strftime("%s")` of it prints `-1`. What macOS's `timegm` refuses
//! beyond that (a minute, hour or day carry that overflows `int`) is its C library's, recorded in
//! `docs/compliance/jq/limitations.md` and pinned here at `succinctly jq`'s own answer, the instant
//! by 64-bit arithmetic, which is also what jq 1.7.1 on Linux (glibc 2.35) answers for those
//! carries (jq 1.8.2 on Linux was not captured).

use std::process::Command;

use anyhow::Result;

use crate::{spawn_with_signal_retry, succinctly_bin};

/// (program, stdout of `try (program) catch .`).
const ROWS: &[(&str, &str)] = &[
    // As jq 1.8.2 (jq 1.7.1 reads this year differently, #7532: `67767976233532800`).
    (
        "[2147485547,12,1,0,0,0,0,0] | mktime",
        "\"invalid gmtime representation\"\n",
    ),
    // As jq 1.8.2 (jq 1.7.1 refuses an array this short).
    (
        "[253402300800,100] | mktime",
        "\"invalid gmtime representation\"\n",
    ),
    // As jq 1.8.2 (jq 1.7.1: `"67767976233532800"`).
    ("[2147485547,12,1,0,0,0,0,0] | strftime(\"%s\")", "\"-1\"\n"),
    (
        "[2147485547,11,31,23,59,59,0,0] | mktime",
        "67768036191676800\n",
    ),
    // Recorded: both macOS releases say `invalid gmtime representation` (tzcode's `int` carries).
    (
        "[1970,0,1,2147483647,2147483647,0,0,0] | mktime",
        "7859790148020\n",
    ),
    ("[1970,0,1,2147483647,60,0,0,0] | mktime", "7730941132800\n"),
    (
        "[1970,0,2147483647,24,0,0,0,0] | mktime",
        "185542587100800\n",
    ),
    // Recorded: the macOS jq 1.8.2 says `67768036191590400` (seconds carried separately).
    (
        "[2147485547,11,31,23,59,60,0,0] | mktime",
        "\"invalid gmtime representation\"\n",
    ),
    // A single saturated field converts in all three.
    ("[1970,0,1,0,2147483647,60,0,0] | mktime", "128849018880\n"),
    ("[1970,0,1,2147483647,0,0,0,0] | mktime", "7730941129200\n"),
];

#[test]
fn mktime_near_the_c_int_limits_7723() -> Result<()> {
    for &(program, want) in ROWS {
        let wrapped = format!("try ({program}) catch .");
        let (output, code) = spawn_with_signal_retry(
            || {
                let mut command = Command::new(succinctly_bin());
                command.env("TZ", "UTC").args(["jq", "-nc", &wrapped]);
                command
            },
            None,
        )?;
        let stdout = String::from_utf8(output.stdout)?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{program}");
    }
    Ok(())
}
