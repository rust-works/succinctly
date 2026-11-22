//! #7783: a POSIX `TZ` is read with any number of digits in an offset's hours, minutes and seconds
//! and in a rule's numbers and times, as both C libraries read it (`XYZ5:30:5`, `XYZ005`,
//! `XYZ+005:00`, `EST5EDT4:5`, `M3.2.0/2:3`), where it was UTC. The `localtime` rows were captured
//! from jq 1.7.1 on macOS (`/usr/bin/jq`) and agree with jq 1.8.2 there and with jq 1.7.1 on glibc;
//! the `%Z %z` rows are jq 1.8.2's (`/opt/homebrew/bin/jq`), since jq 1.7.1's `strflocaltime`
//! labels are its own (#3054). The forms the two C libraries read differently stay as they were and
//! are pinned at `succinctly jq`'s answer (`docs/compliance/jq/limitations.md`).

use std::process::Command;

use anyhow::Result;

use crate::{spawn_with_signal_retry, succinctly_bin};

fn run(tz: &str, program: &str) -> Result<String> {
    let (output, code) = spawn_with_signal_retry(
        || {
            let mut command = Command::new(succinctly_bin());
            command
                .env("TZ", tz)
                .env("LC_ALL", "C")
                .args(["jq", "-nc", program]);
            command
        },
        None,
    )?;
    assert_eq!(code, 0, "TZ={tz} {program}");
    Ok(String::from_utf8(output.stdout)?)
}

const FIELDS: &str = "[0,1720000000] | map(localtime | .[3:6] | map(floor))";
const LABELS: &str = r#"[0,1720000000] | map(strflocaltime("%Z %z"))"#;

/// (TZ, hour/minute/second at 0 and at 1720000000, `%Z %z` at both).
const READ: &[(&str, &str, &str)] = &[
    (
        "XYZ5:30:5",
        "[[18,29,55],[4,16,35]]",
        r#"["XYZ -0530","XYZ -0530"]"#,
    ),
    (
        "XYZ1:5",
        "[[22,55,0],[8,41,40]]",
        r#"["XYZ -0105","XYZ -0105"]"#,
    ),
    (
        "XYZ5:3:5",
        "[[18,56,55],[4,43,35]]",
        r#"["XYZ -0503","XYZ -0503"]"#,
    ),
    (
        "XYZ-0:0:1",
        "[[0,0,1],[9,46,41]]",
        r#"["XYZ +0000","XYZ +0000"]"#,
    ),
    (
        "XYZ005",
        "[[19,0,0],[4,46,40]]",
        r#"["XYZ -0500","XYZ -0500"]"#,
    ),
    (
        "XYZ+005:00",
        "[[19,0,0],[4,46,40]]",
        r#"["XYZ -0500","XYZ -0500"]"#,
    ),
    (
        "XYZ-005",
        "[[5,0,0],[14,46,40]]",
        r#"["XYZ +0500","XYZ +0500"]"#,
    ),
    (
        "XYZ00005:30",
        "[[18,30,0],[4,16,40]]",
        r#"["XYZ -0530","XYZ -0530"]"#,
    ),
    (
        "XYZ024",
        "[[0,0,0],[9,46,40]]",
        r#"["XYZ -2400","XYZ -2400"]"#,
    ),
    (
        "XYZ5:030",
        "[[18,30,0],[4,16,40]]",
        r#"["XYZ -0530","XYZ -0530"]"#,
    ),
    (
        "XYZ5:30:005",
        "[[18,29,55],[4,16,35]]",
        r#"["XYZ -0530","XYZ -0530"]"#,
    ),
    (
        "EST5EDT4:5,M3.2.0,M11.1.0",
        "[[19,0,0],[5,41,40]]",
        r#"["EST -0500","EDT -0405"]"#,
    ),
    (
        "EST5EDT004,M3.2.0,M11.1.0",
        "[[19,0,0],[5,46,40]]",
        r#"["EST -0500","EDT -0400"]"#,
    ),
    (
        "XYZ5:3DDD",
        "[[18,57,0],[5,43,40]]",
        r#"["XYZ -0503","DDD -0403"]"#,
    ),
    (
        "XYZ1:5DDD",
        "[[22,55,0],[9,41,40]]",
        r#"["XYZ -0105","DDD -0005"]"#,
    ),
    (
        "XYZ005DDD,M3.2.0,M11.1.0",
        "[[19,0,0],[5,46,40]]",
        r#"["XYZ -0500","DDD -0400"]"#,
    ),
];

#[test]
fn a_posix_tz_takes_any_digit_count_in_range_7783() -> Result<()> {
    for &(tz, fields, labels) in READ {
        assert_eq!(run(tz, FIELDS)?, format!("{fields}\n"), "TZ={tz}");
        assert_eq!(run(tz, LABELS)?, format!("{labels}\n"), "TZ={tz}");
    }
    Ok(())
}

/// A rule time with one-digit fields moves the transition to that minute and second: either side of
/// 2024-03-10 07:02:30Z/07:03:30Z and 2024-11-03 05:04:50Z/05:05:10Z.
#[test]
fn a_rule_time_takes_any_digit_count_7783() -> Result<()> {
    let program =
        "[1710054150,1710054210,1730610290,1730610310] | map(localtime | .[3:6] | map(floor))";
    for (tz, want) in [
        (
            "EST5EDT,M3.2.0/2:3,M11.1.0/1:5",
            "[[2,2,30],[3,3,30],[1,4,50],[0,5,10]]",
        ),
        (
            "EST5EDT,J60/2:3,J307/1:5",
            "[[3,2,30],[3,3,30],[1,4,50],[0,5,10]]",
        ),
        (
            "EST5EDT,M3.2.0/2:30:5,M11.1.0",
            "[[2,2,30],[2,3,30],[1,4,50],[1,5,10]]",
        ),
    ] {
        assert_eq!(run(tz, program)?, format!("{want}\n"), "TZ={tz}");
    }
    Ok(())
}

/// The forms macOS's libc and glibc read differently, pinned at `succinctly jq`'s reading, UTC: a
/// name of fewer than three characters (POSIX's minimum; glibc reads UTC or drops a short
/// daylight-time name, macOS applies it), a minute of 60 and a trailing `:` (macOS reads UTC, glibc
/// clamps or ignores them), `XYZ:5` (glibc reads UTC, macOS reads `XYZ5`) and a rule time of 168
/// hours (macOS reads UTC, glibc applies it).
#[test]
fn the_forms_the_c_libraries_disagree_on_keep_their_reading_7783() -> Result<()> {
    let utc = "[[0,0,0],[9,46,40]]\n";
    for tz in [
        "XY5",
        "<AB>-5",
        "XYZ5D",
        "XYZ5:60",
        "XYZ5:30:",
        "XYZ:5",
        "EST5EDT,M3.2.0/168,M11.1.0",
    ] {
        assert_eq!(run(tz, FIELDS)?, utc, "TZ={tz}");
    }
    // A signed rule time is read, as glibc reads it (macOS reads the whole `TZ` as UTC).
    assert_eq!(
        run("EST5EDT,M3.2.0/-1,M11.1.0", FIELDS)?,
        "[[19,0,0],[5,46,40]]\n"
    );
    Ok(())
}
