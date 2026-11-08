//! #6690: in jq mode `tz` printed `+03:46` for Australian daylight time (a `13600`-second typo
//! for eleven hours) in `AEDT`, `Australia/Sydney` and `Australia/Melbourne`. The instant is
//! 2023-11-14T22:13:20Z, which is 09:13:20 on the 15th in Sydney; `succinctly yq` already read
//! the zone database and answered `+11:00`.

use crate::run_jq_stdin;
use anyhow::Result;

#[test]
fn tz_prints_eleven_hours_for_australian_daylight_time_6690() -> Result<()> {
    for zone in ["AEDT", "Australia/Sydney", "Australia/Melbourne"] {
        let (out, code) = run_jq_stdin(&format!("tz(\"{zone}\")"), "1700000000", &["-c"])?;
        assert_eq!(
            (out.as_str(), code),
            ("\"2023-11-15T09:13:20+11:00\"\n", 0),
            "{zone}"
        );
    }
    Ok(())
}

/// Standard time and the half-hour zones go through the same formatting.
#[test]
fn tz_prints_australian_standard_and_half_hour_offsets_6690() -> Result<()> {
    // 2024-06-15T12:30:00Z, winter in the southern hemisphere.
    for (zone, expected) in [
        ("AEST", "2024-06-15T22:30:00+10:00"),
        ("Australia/Sydney", "2024-06-15T22:30:00+10:00"),
        ("Australia/Melbourne", "2024-06-15T22:30:00+10:00"),
        ("ACST", "2024-06-15T22:00:00+09:30"),
        ("ACDT", "2024-06-15T23:00:00+10:30"),
        ("Australia/Perth", "2024-06-15T20:30:00+08:00"),
    ] {
        let (out, code) = run_jq_stdin(&format!("tz(\"{zone}\")"), "1718454600", &["-c"])?;
        assert_eq!(
            (out.as_str(), code),
            (format!("\"{expected}\"\n").as_str(), 0),
            "{zone}"
        );
    }
    Ok(())
}
