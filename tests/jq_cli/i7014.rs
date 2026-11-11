//! #7014: `strftime` labels its UTC time as UTC in every zone, as jq 1.8.2 does: `%z` is `+0000`,
//! `%Z` is `UTC` and `%s` is the timestamp it was given. jq 1.7.1 takes the labels from `TZ` and
//! reads the fields back with the local `mktime` for `%s` (`TZ=EST5EDT`, `0 | strftime("%z %Z %s")`
//! is `"-0500 EST 18000"` on macOS and `"+0000 EST 18000"` on glibc). That is a recorded
//! divergence: see `docs/compliance/jq/limitations.md`.
//!
//! The zone-name rows do not depend on a tz file: `strftime` never reads the zone here, so they
//! hold where the zone is missing too.

use anyhow::Result;

use crate::spawn_jq_with_env;

/// `(TZ, input)`; jq 1.7.1's macOS answer in a comment where it differs.
const ROWS: &[(&str, &str)] = &[
    ("UTC", "1700000000"),
    ("EST5EDT", "0"),                   // "-0500 EST 18000"
    ("EST5EDT", "1700000000"),          // "-0500 EST 1700018000"
    ("America/New_York", "1720000000"), // "-0500 EST 1720018000"
    ("Asia/Kolkata", "1720000000"),     // "+0530 IST 1719980200"
];

#[test]
fn strftime_labels_its_time_as_utc_in_every_zone_7014() -> Result<()> {
    for &(zone, input) in ROWS {
        for filter in [
            r#"strftime("%z %Z %s")"#,
            r#"gmtime | strftime("%z %Z %s")"#,
        ] {
            let program = format!("$x | {filter}");
            let (output, code) = spawn_jq_with_env(
                &["-nc", "--argjson", "x", input, &program],
                "TZ",
                zone,
                None,
            )?;
            let stdout = String::from_utf8(output.stdout)?;
            assert_eq!(
                (stdout.as_str(), code),
                (format!("\"+0000 UTC {input}\"\n").as_str(), 0),
                "TZ={zone} {input} | {filter}"
            );
        }
    }
    Ok(())
}
