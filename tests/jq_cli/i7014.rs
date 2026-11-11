//! #7014: `strftime("%s")` prints the timestamp it was given in every zone, as jq 1.8.2 does. jq 1.7.1
//! reads `strftime`'s UTC fields back with the local `mktime`, so away from UTC it shifts the
//! timestamp by the zone's standard offset (`TZ=EST5EDT`, `0 | strftime("%s")` is `"18000"` on macOS
//! and glibc alike). That is a recorded divergence: see `docs/compliance/jq/limitations.md`.
//!
//! The zone-name rows do not depend on a tz file: `strftime` never reads the zone here, so they
//! hold where the zone is missing too.

use std::process::Command;

use anyhow::Result;

use crate::{spawn_with_signal_retry, succinctly_bin};

/// `(TZ, input)`; jq 1.7.1's answer in a comment where it differs.
const ROWS: &[(&str, &str)] = &[
    ("UTC", "1700000000"),
    ("EST5EDT", "0"),                   // jq 1.7.1: "18000"
    ("EST5EDT", "1700000000"),          // jq 1.7.1: "1700018000"
    ("America/New_York", "1720000000"), // jq 1.7.1: "1720018000"
    ("Asia/Kolkata", "1720000000"),     // jq 1.7.1: "1719980200"
];

#[test]
fn strftime_s_is_the_timestamp_in_every_zone_7014() -> Result<()> {
    for &(zone, input) in ROWS {
        for filter in [r#"strftime("%s")"#, r#"gmtime | strftime("%s")"#] {
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
                (format!("\"{input}\"\n").as_str(), 0),
                "TZ={zone} {input} | {filter}"
            );
        }
    }
    Ok(())
}
