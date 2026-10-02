//! The local time zone behind `localtime` and `strflocaltime` (#3054).
//!
//! Every question the date builtins ask about "the local zone" goes through
//! this module, so the zone source is one swappable backend rather than logic
//! spread through the evaluator:
//!
//! - [`offset_at`] -- the UTC offset in effect at a Unix time, for `localtime`.
//! - [`at_instant`] -- the zone in effect at a Unix time with the labels
//!   `strflocaltime` prints, for a number given to it.
//! - [`for_wall_clock`] -- the zone describing a broken-down local time given
//!   directly (an array given to `strflocaltime`), where there is no instant.
//!
//! The zone is a swappable backend. This one reads only a POSIX `TZ` offset
//! string (`EST5EDT`, `UTC-9`) as a fixed offset: an IANA name or an unset `TZ`
//! is UTC, and no daylight-time rule is applied. Under `no_std` there is no
//! environment to read and the answer is UTC.

use alloc::string::String;

/// The zone in effect for one conversion.
pub(crate) struct LocalZone {
    /// Seconds east of UTC at the instant: what `localtime` adds to a Unix
    /// time to reach local fields, and what `%z` prints and `%s` subtracts.
    pub(crate) offset_secs: i64,
    /// The abbreviation `%Z` prints.
    pub(crate) name: String,
}

impl LocalZone {
    /// UTC, labelled `UTC`.
    pub(crate) fn utc() -> Self {
        Self {
            offset_secs: 0,
            name: String::from("UTC"),
        }
    }
}

/// The UTC offset in seconds in effect at `utc_secs` seconds since the Unix
/// epoch -- all `localtime` needs.
#[cfg(feature = "std")] // `no_std` `localtime` is `gmtime`: it has no zone to ask
pub(crate) fn offset_at(utc_secs: i64) -> i64 {
    backend::offset_at(utc_secs)
}

/// The zone in effect at `utc_secs`, with the labels `strflocaltime` prints
/// (`%z`, `%Z`, `%s`).
pub(crate) fn at_instant(utc_secs: i64) -> LocalZone {
    backend::at_instant(utc_secs)
}

/// The zone describing a local wall-clock reading given as fields rather than
/// an instant (`[2024,6,3,18,46,40,3,184] | strflocaltime(...)`). `month` is
/// 1-12; the fields are as given and may be out of range.
pub(crate) fn for_wall_clock(
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
) -> LocalZone {
    backend::for_wall_clock(year, month, day, hour, minute, second)
}

#[cfg(feature = "std")]
use backend_posix as backend;

#[cfg(not(feature = "std"))]
use backend_utc as backend;

#[cfg(not(feature = "std"))]
mod backend_utc {
    use super::LocalZone;

    pub(super) fn at_instant(_utc_secs: i64) -> LocalZone {
        LocalZone::utc()
    }

    pub(super) fn for_wall_clock(
        _year: i64,
        _month: i64,
        _day: i64,
        _hour: i64,
        _minute: i64,
        _second: i64,
    ) -> LocalZone {
        LocalZone::utc()
    }
}

/// A POSIX `TZ` offset string (`EST5EDT`, `UTC-9`) read at the instant of the
/// call: a fixed offset and the standard-time abbreviation, whatever the
/// timestamp. An IANA zone name or an unset `TZ` is not understood and is UTC.
#[cfg(feature = "std")]
mod backend_posix {
    use super::LocalZone;
    use alloc::string::String;
    use alloc::vec::Vec;

    pub(super) fn offset_at(_utc_secs: i64) -> i64 {
        current().offset_secs
    }

    pub(super) fn at_instant(_utc_secs: i64) -> LocalZone {
        current()
    }

    pub(super) fn for_wall_clock(
        _year: i64,
        _month: i64,
        _day: i64,
        _hour: i64,
        _minute: i64,
        _second: i64,
    ) -> LocalZone {
        current()
    }

    fn current() -> LocalZone {
        if let Ok(tz) = std::env::var("TZ") {
            if let Some(offset_secs) = parse_simple_tz_offset(&tz) {
                let name: String = tz.chars().take_while(char::is_ascii_alphabetic).collect();
                return LocalZone {
                    offset_secs,
                    name: if name.is_empty() {
                        String::from("UTC")
                    } else {
                        name
                    },
                };
            }
        }
        LocalZone::utc()
    }

    /// Parse a simple TZ offset like "EST5" or "PST8" and return offset in seconds
    pub(super) fn parse_simple_tz_offset(tz: &str) -> Option<i64> {
        // Skip the timezone name (letters)
        let offset_start = tz.find(|c: char| c.is_ascii_digit() || c == '-' || c == '+')?;
        let offset_part = &tz[offset_start..];

        // Find where the offset ends (at DST name or end of string)
        let offset_end = offset_part
            .find(|c: char| c.is_ascii_alphabetic())
            .unwrap_or(offset_part.len());
        let offset_str = &offset_part[..offset_end];

        // Parse the offset (hours, optionally minutes)
        let negative = offset_str.starts_with('-');
        let offset_str = offset_str.trim_start_matches(['+', '-']);

        let parts: Vec<&str> = offset_str.split(':').collect();
        let hours: i64 = parts.first()?.parse().ok()?;
        let minutes: i64 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);

        // TZ offset is positive for west of UTC, but we want seconds to add.
        // `hours`/`minutes` are parsed straight out of the `TZ` env var with no
        // bound (#894) — checked throughout so a malformed/adversarial `TZ`
        // falls back to the existing `None` -> UTC path instead of panicking.
        let hour_secs = hours.checked_mul(3600)?;
        let minute_secs = minutes.checked_mul(60)?;
        let offset_secs = hour_secs
            .checked_add(minute_secs)?
            .checked_mul(if negative { 1 } else { -1 })?;
        Some(offset_secs)
    }

    #[cfg(test)]
    mod tests {
        use super::parse_simple_tz_offset;

        #[test]
        fn test_parse_simple_tz_offset_errors_gracefully_on_overflow_894() {
            // #894: `hours * 3600`/`minutes * 60`/the final sign multiply were
            // unchecked, panicking on a malformed/adversarial `TZ` env var
            // (reachable independently of the timestamp being converted, since
            // `TZ` parsing happens before any timestamp arithmetic runs). Tested
            // directly against the private helper rather than through the `TZ`
            // env var + `localtime` builtin, since mutating process-global env
            // vars in a parallel test binary is inherently racy.
            assert_eq!(parse_simple_tz_offset("EST9999999999999999"), None); // hours overflow
            assert_eq!(parse_simple_tz_offset("EST-9999999999999999"), None); // hours overflow, negative
            assert_eq!(
                parse_simple_tz_offset("EST1:999999999999999999"),
                None // minutes overflow (hours alone is in range)
            );

            // Normal input still resolves correctly (positive TZ = west of UTC
            // = negative offset applied, per this function's own doc comment).
            assert_eq!(parse_simple_tz_offset("EST5EDT"), Some(-5 * 3600));
            assert_eq!(parse_simple_tz_offset("EST-5EDT"), Some(5 * 3600));
        }
    }
}
