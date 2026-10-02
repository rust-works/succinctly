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
//! The backends, from most to least capable:
//!
//! - `local-zone` (on in `cli`, not on Windows): [`jiff`](https://docs.rs/jiff).
//!   `TZ` as an IANA name or a POSIX string, otherwise the system zone, with
//!   the offset *and abbreviation* in effect at the instant, so daylight time
//!   follows the date and `%z`, `%Z` and `%s` are the zone's own. `TZ` is read
//!   once per process, on the first conversion; the POSIX-only backend below
//!   re-reads it on every call.
//! - bare `std` (and Windows): only a POSIX `TZ` offset string, read as a fixed
//!   offset; an IANA name or an unset `TZ` is UTC.
//! - `no_std`: UTC.
//!
//! The labels are the *true* ones, not jq 1.7.1's: that jq labels a
//! daylight-time instant with its zone's standard offset and abbreviation (and
//! on glibc prints `+0000` for `%z` in every zone); jq 1.8.2 prints the true
//! values this module does. See `docs/compliance/jq/limitations.md`.

use alloc::string::String;

/// The zone in effect for one conversion.
pub(crate) struct LocalZone {
    /// Seconds east of UTC at the instant: what `localtime` adds to a Unix
    /// time to reach local fields, and what `%z` prints and `%s` subtracts.
    pub(crate) offset_secs: i64,
    /// The abbreviation `%Z` prints (`EDT`, `JST`; `+0530` for a zone with no
    /// letters of its own).
    pub(crate) name: String,
}

impl LocalZone {
    /// UTC, labelled `UTC`.
    #[cfg(not(all(feature = "std", feature = "local-zone", not(windows))))]
    pub(crate) fn utc() -> Self {
        Self {
            offset_secs: 0,
            name: String::from("UTC"),
        }
    }
}

/// The UTC offset in seconds in effect at `utc_secs` seconds since the Unix
/// epoch -- all `localtime` needs. Every instant has one: where a backend's
/// calendar ends, it answers for the nearest instant it can place.
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
/// 1-12; the fields are as given, may be out of range, and are normalised the
/// way C's `mktime` does (month 13 is January of the next year, hour 25 is
/// 01:00 the next day). A reading the zone skipped or repeated resolves to the
/// earlier of the two instants for a repeat and the later for a skip.
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

#[cfg(all(feature = "std", feature = "local-zone", not(windows)))]
use backend_jiff as backend;

#[cfg(all(feature = "std", not(all(feature = "local-zone", not(windows)))))]
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

/// The system's local zone through `jiff`: `TZ` as an IANA name, a `:name`, a
/// path or a POSIX string, otherwise the system zone. `TZ` is read once per
/// process, on the first conversion (a later `set_var` is not seen). A `TZ` it
/// cannot read is UTC, as in jq.
///
/// One case jiff declines and this module covers: a POSIX string with a
/// daylight-time name but no rule (`EST5EDT`) whose name is not a tz file (newer
/// distributions no longer ship the legacy `EST5EDT`/`PST8PDT` files). POSIX
/// leaves the rule unspecified; libc applies the US rule, which is what these
/// names mean, so it is applied here too.
#[cfg(all(feature = "std", feature = "local-zone", not(windows)))]
mod backend_jiff {
    use super::LocalZone;
    use alloc::format;
    use alloc::string::ToString;
    use jiff::civil::DateTime;
    use jiff::tz::TimeZone;
    use jiff::{Span, Timestamp};
    use std::sync::OnceLock;

    /// The US rule, `M<month>.<week>.<weekday>`: second Sunday of March to
    /// first Sunday of November (at 02:00 local time, the default).
    const DEFAULT_RULE: &str = "M3.2.0,M11.1.0";

    fn system() -> &'static TimeZone {
        static ZONE: OnceLock<TimeZone> = OnceLock::new();
        ZONE.get_or_init(|| {
            std::env::var("TZ")
                .ok()
                .as_deref()
                .and_then(with_default_rule)
                .unwrap_or_else(TimeZone::system)
        })
    }

    pub(super) fn offset_at(utc_secs: i64) -> i64 {
        offset_in(system(), utc_secs)
    }

    pub(super) fn at_instant(utc_secs: i64) -> LocalZone {
        zone_in(system(), utc_secs)
    }

    pub(super) fn for_wall_clock(
        year: i64,
        month: i64,
        day: i64,
        hour: i64,
        minute: i64,
        second: i64,
    ) -> LocalZone {
        wall_clock_in(system(), year, month, day, hour, minute, second)
    }

    /// The Gregorian calendar repeats every 400 years: 146097 days, a whole
    /// number of weeks, so the weekday, the day of the year and the leap-year
    /// pattern recur.
    const CYCLE_SECS: i64 = 146_097 * 86_400;

    /// jiff's calendar ends at +-9999. Past the end of it the zone's rule is the
    /// POSIX rule of the last year, which depends only on the date within the
    /// cycle, so the instant is folded back by whole 400-year cycles to the same
    /// position inside the calendar -- the answer libc gives for year 33658.
    /// Before its start a zone has no transitions, so the earliest instant
    /// answers, as it does in libc.
    fn timestamp(utc_secs: i64) -> Timestamp {
        if let Ok(ts) = Timestamp::from_second(utc_secs) {
            return ts;
        }
        if utc_secs < 0 {
            return Timestamp::MIN;
        }
        let over = utc_secs - Timestamp::MAX.as_second();
        let cycles = (over + CYCLE_SECS - 1) / CYCLE_SECS;
        Timestamp::from_second(utc_secs - cycles * CYCLE_SECS).unwrap_or(Timestamp::MAX)
    }

    fn offset_in(tz: &TimeZone, utc_secs: i64) -> i64 {
        i64::from(tz.to_offset(timestamp(utc_secs)).seconds())
    }

    fn zone_in(tz: &TimeZone, utc_secs: i64) -> LocalZone {
        let info = tz.to_offset_info(timestamp(utc_secs));
        LocalZone {
            offset_secs: i64::from(info.offset().seconds()),
            name: info.abbreviation().to_string(),
        }
    }

    fn wall_clock_in(
        tz: &TimeZone,
        year: i64,
        month: i64,
        day: i64,
        hour: i64,
        minute: i64,
        second: i64,
    ) -> LocalZone {
        // January 1st plus the fields as a span normalises them as `mktime`
        // would; jiff rejects a span or a date beyond its calendar, and a
        // reading that cannot be placed is labelled by the zone at the epoch.
        let civil = (|| {
            let span = Span::new()
                .try_months(month.checked_sub(1)?)
                .ok()?
                .try_days(day.checked_sub(1)?)
                .ok()?
                .try_hours(hour)
                .ok()?
                .try_minutes(minute)
                .ok()?
                .try_seconds(second)
                .ok()?;
            DateTime::new(i16::try_from(year).ok()?, 1, 1, 0, 0, 0, 0)
                .ok()?
                .checked_add(span)
                .ok()
        })();
        let ts = civil
            .and_then(|dt| tz.to_ambiguous_timestamp(dt).compatible().ok())
            .map_or(0, Timestamp::as_second);
        zone_in(tz, ts)
    }

    /// The zone `tz` names when it is a rule-less POSIX string with no tz file
    /// of that name: the string plus [`DEFAULT_RULE`]. `None` otherwise, which
    /// leaves `tz` to jiff.
    fn with_default_rule(tz: &str) -> Option<TimeZone> {
        if !is_rule_less_posix(tz) || jiff::tz::db().get(tz).is_ok() {
            return None;
        }
        TimeZone::posix(&format!("{tz},{DEFAULT_RULE}")).ok()
    }

    /// Whether `tz` is `std offset dst [offset]` with no `,rule` after it
    /// (`EST5EDT`, `CET-1CEST`, `<-03>3<-02>`), the form POSIX leaves without a
    /// rule. A name is three or more letters or a `<...>` quoted name; an
    /// offset is `[+-]hh[:mm[:ss]]`.
    pub(super) fn is_rule_less_posix(tz: &str) -> bool {
        fn name(s: &str) -> Option<&str> {
            if let Some(quoted) = s.strip_prefix('<') {
                let end = quoted.find('>')?;
                return Some(&quoted[end + 1..]);
            }
            let letters = s
                .find(|c: char| !c.is_ascii_alphabetic())
                .unwrap_or(s.len());
            (letters >= 3).then(|| &s[letters..])
        }
        fn number(s: &str) -> Option<&str> {
            let digits = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
            (1..=2).contains(&digits).then(|| &s[digits..])
        }
        fn offset(s: &str) -> Option<&str> {
            let mut rest = number(s.strip_prefix(['+', '-']).unwrap_or(s))?;
            for _ in 0..2 {
                match rest.strip_prefix(':') {
                    Some(after) => rest = number(after)?,
                    None => break,
                }
            }
            Some(rest)
        }
        let Some(after_dst_name) = name(tz).and_then(offset).and_then(name) else {
            return false;
        };
        after_dst_name.is_empty() || offset(after_dst_name).is_some_and(str::is_empty)
    }

    #[cfg(test)]
    mod tests {
        use super::{
            is_rule_less_posix, offset_in, wall_clock_in, with_default_rule, zone_in, TimeZone,
        };

        /// New York's rule as a POSIX string: needs no tz database on the
        /// machine running the test.
        fn new_york() -> TimeZone {
            TimeZone::posix("EST5EDT,M3.2.0,M11.1.0").unwrap()
        }

        #[test]
        fn offset_and_abbreviation_are_those_in_effect_at_the_instant() {
            let ny = new_york();
            // 2024-01-11 19:06:40Z and 2024-07-03 09:46:40Z.
            assert_eq!(offset_in(&ny, 1_705_000_000), -5 * 3600);
            assert_eq!(offset_in(&ny, 1_720_000_000), -4 * 3600);
            let winter = zone_in(&ny, 1_705_000_000);
            assert_eq!((winter.offset_secs, winter.name.as_str()), (-18_000, "EST"));
            let summer = zone_in(&ny, 1_720_000_000);
            assert_eq!((summer.offset_secs, summer.name.as_str()), (-14_400, "EDT"));
            // Either side of the 2024 spring-forward (07:00Z) and fall-back (06:00Z).
            assert_eq!(offset_in(&ny, 1_710_053_999), -5 * 3600);
            assert_eq!(offset_in(&ny, 1_710_054_000), -4 * 3600);
            assert_eq!(offset_in(&ny, 1_730_613_599), -4 * 3600);
            assert_eq!(offset_in(&ny, 1_730_613_600), -5 * 3600);
        }

        #[test]
        fn an_instant_past_the_calendar_keeps_the_zones_rule() {
            let ny = new_york();
            // 1e12 s is 33658-09-27 and 100 days on is January 33659: the US rule
            // still puts New York on daylight time in the first and standard
            // time in the second, as libc does.
            assert_eq!(offset_in(&ny, 1_000_000_000_000), -4 * 3600);
            assert_eq!(offset_in(&ny, 1_000_000_000_000 + 100 * 86_400), -5 * 3600);
            // Folding by whole 400-year cycles lands on the same position in the
            // calendar, so it is the same answer as the folded instant gives.
            let cycle = 146_097 * 86_400;
            assert_eq!(
                offset_in(&ny, 1_000_000_000_000),
                offset_in(&ny, 1_000_000_000_000 - 60 * cycle)
            );
            // Before the calendar: the earliest instant's offset.
            assert_eq!(
                offset_in(&ny, -1_000_000_000_000_000),
                offset_in(&ny, jiff::Timestamp::MIN.as_second())
            );
        }

        #[test]
        fn an_instant_past_either_end_of_the_calendar_does_not_panic() {
            let ny = new_york();
            for secs in [
                i64::MIN,
                i64::MAX,
                -1_000_000_000_000_000,
                1_000_000_000_000_000,
            ] {
                let _ = offset_in(&ny, secs);
                let _ = zone_in(&ny, secs);
            }
        }

        #[test]
        fn a_wall_clock_reading_is_resolved_the_way_mktime_would() {
            let ny = new_york();
            let at = |y, mo, d, h, mi, s| {
                let zone = wall_clock_in(&ny, y, mo, d, h, mi, s);
                (zone.offset_secs, zone.name)
            };
            assert_eq!(at(2024, 1, 11, 14, 6, 40), (-18_000, "EST".into()));
            assert_eq!(at(2024, 7, 3, 5, 46, 40), (-14_400, "EDT".into()));
            // 02:30 on 2024-03-10 never happened: the later reading, daylight time.
            assert_eq!(at(2024, 3, 10, 2, 30, 0), (-14_400, "EDT".into()));
            // 01:30 on 2024-11-03 happened twice: the earlier reading, daylight time.
            assert_eq!(at(2024, 11, 3, 1, 30, 0), (-14_400, "EDT".into()));
            // Fields are normalised: month 13 is January 2025, hour 25 is 01:00 next
            // day, day 0 is the last day of the previous month.
            assert_eq!(at(2024, 13, 1, 0, 0, 0), (-18_000, "EST".into()));
            assert_eq!(at(2024, 7, 3, 25, 0, 0), (-14_400, "EDT".into()));
            assert_eq!(at(2024, 8, 0, 12, 0, 0), (-14_400, "EDT".into()));
        }

        #[test]
        fn a_wall_clock_reading_that_cannot_be_placed_uses_the_zone_at_the_epoch() {
            let ny = new_york();
            for (year, month) in [
                (i64::MAX, 1),
                (2024, i64::MAX),
                (2024, i64::MIN),
                (1_000_000, 1),
            ] {
                let zone = wall_clock_in(&ny, year, month, 1, 0, 0, 0);
                assert_eq!((zone.offset_secs, zone.name.as_str()), (-18_000, "EST"));
            }
            // A day or an hour too large for a span, not just a month.
            let zone = wall_clock_in(&ny, 2024, 7, i64::MAX, 0, 0, 0);
            assert_eq!(zone.offset_secs, -18_000);
        }

        #[test]
        fn a_rule_less_posix_string_is_recognised() {
            for tz in [
                "EST5EDT",
                "PST8PDT",
                "CET-1CEST",
                "XYZ5XYD",
                "AAA-3:30BBB",
                "<-03>3<-02>",
                "EST5EDT4",
                "EST5:30EDT",
            ] {
                assert!(is_rule_less_posix(tz), "{tz}");
            }
            for tz in [
                "",
                "UTC",
                "EST5",
                "UTC-9",
                "Asia/Tokyo",
                "America/New_York",
                "Etc/GMT+5",
                "EST5EDT,M3.2.0,M11.1.0",
                "EST5EDT,J60,J300",
                "AB5CD",
                "EST5EDT4x",
                "EST123EDT",
                "5",
            ] {
                assert!(!is_rule_less_posix(tz), "{tz}");
            }
        }

        #[test]
        fn the_default_rule_applies_only_to_a_rule_less_string_with_no_tz_file() {
            // No such tz file: the US rule.
            let zone = with_default_rule("XYZ5XYD").expect("rule-less, no tz file");
            assert_eq!(offset_in(&zone, 1_705_000_000), -5 * 3600);
            assert_eq!(offset_in(&zone, 1_720_000_000), -4 * 3600);
            assert_eq!(zone_in(&zone, 1_720_000_000).name, "XYD");
            // Left to jiff: no daylight time, an explicit rule, an IANA name.
            for tz in ["UTC-9", "EST5", "EST5EDT,M3.2.0,M11.1.0", "Asia/Tokyo"] {
                assert!(with_default_rule(tz).is_none(), "{tz}");
            }
        }
    }
}

/// A POSIX `TZ` offset string (`EST5EDT`, `UTC-9`) read at the instant of the
/// call: a fixed offset and the standard-time abbreviation, whatever the
/// timestamp. An IANA zone name or an unset `TZ` is not understood and is UTC.
/// Also what Windows keeps when `local-zone` is on.
#[cfg(all(feature = "std", not(all(feature = "local-zone", not(windows)))))]
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
