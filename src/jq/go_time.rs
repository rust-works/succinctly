//! Go's `time.Parse` for the two layouts yq reads a timestamp with (#5651): `time.RFC3339`, then
//! `2006-01-02`.
//!
//! yq's ordering comparison (`compareOperator`, `pkg/yqlib/operator_compare.go`, v4.53.3) reads a
//! left operand whose text parses under either layout as a time, and then parses the right
//! operand's text the same way -- so `"2001-01-01" < 1` raises Go's own
//! `parsing time "1" as "2006-01-02": cannot parse "1" as "2006"`. Only success or failure and the
//! text of the *date* layout's error matter: that layout is the last one tried, and yq reports the
//! last error.
//!
//! This ports the part of `time.Parse` (`time/format.go`, Go 1.24) those two layouts exercise,
//! including its looser-than-RFC 3339 reading (a one-digit hour, a comma before the fraction, a
//! zone offset of 24 hours), because which strings count as timestamps decides whether a
//! comparison is made as times at all. `scripts/yq-time-fuzz.py` replays the same strings against
//! a real yq.

use alloc::format;
use alloc::string::String;

/// A parsed instant, as nanoseconds since the Unix epoch.
pub(crate) type Instant = i128;

/// The instant yq reads `text` as, or the error yq reports when neither layout parses it.
pub(crate) fn parse_yq_datetime(text: &str) -> Result<Instant, String> {
    match parse_rfc3339(text.as_bytes()) {
        Some(instant) => Ok(instant),
        None => parse_date(text.as_bytes()),
    }
}

fn is_digit(s: &[u8], i: usize) -> bool {
    s.get(i).is_some_and(u8::is_ascii_digit)
}

/// Go's `getnum`: one or two digits, or exactly two when `fixed`.
fn getnum(s: &[u8], fixed: bool) -> Option<(i64, &[u8])> {
    if !is_digit(s, 0) {
        return None;
    }
    if !is_digit(s, 1) {
        return if fixed {
            None
        } else {
            Some((i64::from(s[0] - b'0'), &s[1..]))
        };
    }
    Some((
        i64::from(s[0] - b'0') * 10 + i64::from(s[1] - b'0'),
        &s[2..],
    ))
}

fn is_leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_in(month: i64, year: i64) -> i64 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days from 1970-01-01 to the proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn instant(
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
    nanos: i64,
    offset_seconds: i64,
) -> Instant {
    let seconds = days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second
        - offset_seconds;
    i128::from(seconds) * 1_000_000_000 + i128::from(nanos)
}

/// `time.Parse(time.RFC3339, text)`: `2006-01-02T15:04:05Z07:00`, read as Go reads it.
///
/// Only whether it parses, and the instant, are returned: its error is never the one yq reports.
fn parse_rfc3339(text: &[u8]) -> Option<Instant> {
    let mut v = text;
    // 2006
    if v.len() < 4 || !is_digit(v, 0) || !v[..4].iter().all(u8::is_ascii_digit) {
        return None;
    }
    let year = v[..4]
        .iter()
        .fold(0i64, |n, d| n * 10 + i64::from(d - b'0'));
    v = &v[4..];
    // -01
    v = v.strip_prefix(b"-")?;
    let (month, rest) = getnum(v, true)?;
    if !(1..=12).contains(&month) {
        return None;
    }
    v = rest.strip_prefix(b"-")?;
    // 02: any two-digit day here; the combination is checked after the whole layout is read.
    let (day, rest) = getnum(v, true)?;
    v = rest.strip_prefix(b"T")?;
    // 15 (one or two digits), :04, :05
    let (hour, rest) = getnum(v, false)?;
    if !(0..24).contains(&hour) {
        return None;
    }
    v = rest.strip_prefix(b":")?;
    let (minute, rest) = getnum(v, true)?;
    if !(0..60).contains(&minute) {
        return None;
    }
    v = rest.strip_prefix(b":")?;
    let (second, rest) = getnum(v, true)?;
    if !(0..60).contains(&second) {
        return None;
    }
    v = rest;
    // A fraction after the seconds, though the layout has none: a period or a comma, then
    // digits, of which the first nine count.
    let mut nanos = 0i64;
    if v.len() >= 2 && (v[0] == b'.' || v[0] == b',') && is_digit(v, 1) {
        let mut end = 2;
        while is_digit(v, end) {
            end += 1;
        }
        let digits = &v[1..end.min(10)];
        nanos = digits
            .iter()
            .fold(0i64, |n, d| n * 10 + i64::from(d - b'0'));
        for _ in digits.len()..9 {
            nanos *= 10;
        }
        v = &v[end..];
    }
    // Z07:00
    let offset = if let Some(rest) = v.strip_prefix(b"Z") {
        v = rest;
        0
    } else {
        if v.len() < 6 || v[3] != b':' {
            return None;
        }
        let (hours, _) = getnum(&v[1..3], true)?;
        let (minutes, _) = getnum(&v[4..6], true)?;
        // Go allows 24 hours and 60 minutes here: some people write them.
        if hours > 24 || minutes > 60 {
            return None;
        }
        let seconds = hours * 3_600 + minutes * 60;
        let signed = match v[0] {
            b'+' => seconds,
            b'-' => -seconds,
            _ => return None,
        };
        v = &v[6..];
        signed
    };
    if !v.is_empty() || day < 1 || day > days_in(month, year) {
        return None;
    }
    Some(instant(
        year, month, day, hour, minute, second, nanos, offset,
    ))
}

/// Go's `time.quote`: a quoted string with `"` and `\` escaped and every byte that is not
/// printable ASCII written as `\xNN`.
fn quote(bytes: &[u8]) -> String {
    let mut out = String::from("\"");
    for &b in bytes {
        if b == b'"' || b == b'\\' {
            out.push('\\');
            out.push(char::from(b));
        } else if b >= 0x80 || b < b' ' {
            out.push_str(&format!("\\x{b:02x}"));
        } else {
            out.push(char::from(b));
        }
    }
    out.push('"');
    out
}

/// `time.Parse("2006-01-02", text)`, with Go's `ParseError` text for a failure.
fn parse_date(text: &[u8]) -> Result<Instant, String> {
    let whole = quote(text);
    // `cannot parse "<rest>" as "<element>"`.
    let element = |rest: &[u8], layout: &str| {
        format!(
            "parsing time {whole} as \"2006-01-02\": cannot parse {} as \"{layout}\"",
            quote(rest)
        )
    };
    let mut v = text;
    // 2006
    let hold = v;
    if v.len() < 4 || !is_digit(v, 0) || !v[..4].iter().all(u8::is_ascii_digit) {
        return Err(element(hold, "2006"));
    }
    let year = v[..4]
        .iter()
        .fold(0i64, |n, d| n * 10 + i64::from(d - b'0'));
    v = &v[4..];
    // -
    v = v.strip_prefix(b"-").ok_or_else(|| element(v, "-"))?;
    // 01
    let hold = v;
    let (month, rest) = getnum(v, true).ok_or_else(|| element(hold, "01"))?;
    v = rest;
    if !(1..=12).contains(&month) {
        return Err(format!("parsing time {whole}: month out of range"));
    }
    // -
    v = v.strip_prefix(b"-").ok_or_else(|| element(v, "-"))?;
    // 02
    let hold = v;
    let (day, rest) = getnum(v, true).ok_or_else(|| element(hold, "02"))?;
    v = rest;
    if !v.is_empty() {
        return Err(format!("parsing time {whole}: extra text: {}", quote(v)));
    }
    if day < 1 || day > days_in(month, year) {
        return Err(format!("parsing time {whole}: day out of range"));
    }
    Ok(instant(year, month, day, 0, 0, 0, 0, 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(text: &str) -> Instant {
        parse_yq_datetime(text).unwrap_or_else(|e| panic!("{text}: {e}"))
    }

    fn err(text: &str) -> String {
        parse_yq_datetime(text).expect_err(text)
    }

    /// Every row captured from yq v4.53.3 as `"<text>" < "x"` (an error when the text is a time).
    #[test]
    fn which_texts_are_timestamps_matches_yq_5651() {
        for text in [
            "2001-01-01T00:00:00Z",
            "2001-01-01T00:00:00+10:00",
            "2001-01-01T00:00:00.5Z",
            "2001-01-01",
            "0001-01-01",
            "2001-01-01T00:00:00-00:00",
        ] {
            assert!(parse_yq_datetime(text).is_ok(), "{text}");
        }
        for text in [
            "2001-01-01T00:00:00",
            "2001-01-01 00:00:00",
            "2001-1-1",
            "20010101",
            "2001-01-01T00:00Z",
            "Mon, 02 Jan 2006 15:04:05 MST",
            "2001-01-01T00:00:00 Z",
            " 2001-01-01",
            "2001-01-01 ",
            "2001-13-01",
            "2001-01-01T24:00:00Z",
            "2001-02-30",
            "2001-01-01t00:00:00z",
        ] {
            assert!(parse_yq_datetime(text).is_err(), "{text}");
        }
    }

    /// The error texts yq printed for the right operand, from the same probes.
    #[test]
    fn the_date_layouts_error_text_matches_yq_5651() {
        for (text, want) in [
            (
                "1",
                r#"parsing time "1" as "2006-01-02": cannot parse "1" as "2006""#,
            ),
            (
                "",
                r#"parsing time "" as "2006-01-02": cannot parse "" as "2006""#,
            ),
            (
                "1.5",
                r#"parsing time "1.5" as "2006-01-02": cannot parse "1.5" as "2006""#,
            ),
            (
                "1_000",
                r#"parsing time "1_000" as "2006-01-02": cannot parse "1_000" as "2006""#,
            ),
            (
                "null",
                r#"parsing time "null" as "2006-01-02": cannot parse "null" as "2006""#,
            ),
            (
                "20010101",
                r#"parsing time "20010101" as "2006-01-02": cannot parse "0101" as "-""#,
            ),
            (
                "2002-13-01",
                r#"parsing time "2002-13-01": month out of range"#,
            ),
            (
                "2001-01-01 00:00:00",
                r#"parsing time "2001-01-01 00:00:00": extra text: " 00:00:00""#,
            ),
            (
                "2001-01-01T00:00:00",
                r#"parsing time "2001-01-01T00:00:00": extra text: "T00:00:00""#,
            ),
            (
                "2001-02-30",
                r#"parsing time "2001-02-30": day out of range"#,
            ),
        ] {
            assert_eq!(err(text), want, "{text}");
        }
    }

    #[test]
    fn instants_order_by_the_moment_not_the_text_5651() {
        // The same moment in two zones is equal, and orders by the instant, not the string.
        assert_eq!(ok("2001-01-01T00:00:00Z"), ok("2001-01-01T10:00:00+10:00"));
        assert!(ok("2001-01-01T00:00:00Z") < ok("2001-01-01T00:00:01Z"));
        assert_eq!(ok("2001-01-01"), ok("2001-01-01T00:00:00Z"));
        assert!(ok("2000-12-31") < ok("2001-01-01"));
        assert!(ok("2001-01-01T00:00:00.5Z") > ok("2001-01-01T00:00:00Z"));
    }

    #[test]
    fn go_reads_rfc3339_more_loosely_than_the_standard_5651() {
        // A one-digit hour, a comma fraction and a 24-hour zone offset all parse in Go.
        assert!(parse_yq_datetime("2001-01-01T1:00:00Z").is_ok());
        assert!(parse_yq_datetime("2001-01-01T00:00:00,5Z").is_ok());
        assert!(parse_yq_datetime("2001-01-01T00:00:00+24:00").is_ok());
        // But not a fraction with nothing after the separator, or a 25-hour offset.
        assert!(parse_yq_datetime("2001-01-01T00:00:00.Z").is_err());
        assert!(parse_yq_datetime("2001-01-01T00:00:00+25:00").is_err());
    }

    #[test]
    fn a_non_ascii_or_control_byte_is_written_as_go_quotes_it_5651() {
        assert_eq!(
            err("é"),
            r#"parsing time "\xc3\xa9" as "2006-01-02": cannot parse "\xc3\xa9" as "2006""#
        );
        assert_eq!(
            err("a\"b"),
            r#"parsing time "a\"b" as "2006-01-02": cannot parse "a\"b" as "2006""#
        );
    }
}
