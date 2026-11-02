//! Go's `time` package as far as yq uses it: `time.Parse` and `time.Format` over any layout,
//! `time.ParseDuration`, and the two layouts yq reads a timestamp with by default (#5651, #5863).
//!
//! yq keeps a timestamp as text tagged `!!timestamp` and does every datetime operation on that
//! text: it parses it with `time.Parse` under the current layout (`time.RFC3339` unless
//! `with_dtf` set another, and `2006-01-02` when RFC 3339 fails), works on the `time.Time`, and
//! writes the result back with `Format(layout)`. So which strings parse, what the error says and
//! what a formatted time spells all come from Go, and are ported here from `time/format.go`
//! (Go 1.24), including its looser-than-RFC 3339 reading (a one-digit hour, a comma before the
//! fraction, a zone offset of 24 hours).
//!
//! yq's ordering comparison (`compareOperator`, `pkg/yqlib/operator_compare.go`, v4.53.3) reads a
//! left operand whose text parses under either default layout as a time, and then parses the
//! right operand's text the same way -- so `"2001-01-01" < 1` raises Go's own
//! `parsing time "1" as "2006-01-02": cannot parse "1" as "2006"`. [`yq_datetime`] answers that
//! question without allocating. `scripts/yq-time-fuzz.py` replays mutated timestamps against a
//! real yq.
//!
//! A time's zone is one of Go's three kinds of `Location`: UTC, the local zone, or a fixed zone
//! made up by `Parse` for an offset the local zone does not have at that instant. The local zone
//! is asked through [`Location`], so this module needs no zone database of its own.

use alloc::format;
use alloc::string::String;

/// A parsed instant, as nanoseconds since the Unix epoch.
pub(crate) type Instant = i128;

/// `time.RFC3339`, yq's default layout.
pub(crate) const RFC3339: &str = "2006-01-02T15:04:05Z07:00";

/// The layout yq falls back to when RFC 3339 does not parse.
const DATE: &str = "2006-01-02";

const NANOS_PER_SECOND: i128 = 1_000_000_000;

/// The local zone, as Go's `time.Local` answers for it.
pub(crate) trait Location {
    /// Go's `Location.lookup`: the abbreviation and the offset in seconds east of UTC in effect
    /// at `unix` seconds since the epoch.
    fn lookup(&self, unix: i64) -> (String, i64);

    /// Go's `Location.lookupName`: the offset of a zone the location calls `name`, preferring
    /// one in effect at the wall clock `unix` (read as if it were UTC).
    fn lookup_name(&self, name: &str, unix: i64) -> Option<i64>;
}

/// A local zone that is UTC, which is what Go's `time.Local` is with no zone configured.
pub(crate) struct UtcLocation;

impl Location for UtcLocation {
    fn lookup(&self, _unix: i64) -> (String, i64) {
        (String::from("UTC"), 0)
    }

    fn lookup_name(&self, name: &str, _unix: i64) -> Option<i64> {
        (name == "UTC").then_some(0)
    }
}

/// The zone a [`GoTime`] is in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Zone {
    /// `time.UTC`, named `UTC`.
    Utc,
    /// `time.Local`: its offset and name are looked up at the instant.
    Local,
    /// A `time.FixedZone`, whose name may be empty.
    Fixed { name: String, offset: i64 },
}

/// A Go `time.Time`: an instant and the zone it is shown in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GoTime {
    nanos: Instant,
    zone: Zone,
}

impl GoTime {
    /// The time `nanos` nanoseconds after the epoch, in `zone`.
    pub(crate) fn new(nanos: Instant, zone: Zone) -> Self {
        Self { nanos, zone }
    }

    /// `t.Add(d)`: the same zone, `d` nanoseconds later.
    pub(crate) fn add(&self, duration: i64) -> Self {
        Self {
            nanos: self.nanos + i128::from(duration),
            zone: self.zone.clone(),
        }
    }

    /// `t.Unix()`: whole seconds since the epoch, rounded down.
    pub(crate) fn unix(&self) -> i64 {
        // An instant parsed from text has a year of at most four digits, so it fits.
        self.nanos.div_euclid(NANOS_PER_SECOND) as i64
    }

    /// The instant, in nanoseconds since the epoch.
    pub(crate) fn instant(&self) -> Instant {
        self.nanos
    }

    /// `t.In(loc)`: the same instant shown in `zone`.
    pub(crate) fn in_zone(&self, zone: Zone) -> Self {
        Self {
            nanos: self.nanos,
            zone,
        }
    }

    /// The zone's name and offset at this instant, as `Format` shows them.
    fn zone_at(&self, local: &dyn Location) -> (String, i64) {
        match &self.zone {
            Zone::Utc => (String::from("UTC"), 0),
            Zone::Local => local.lookup(self.unix()),
            Zone::Fixed { name, offset } => (name.clone(), *offset),
        }
    }
}

/// The instant yq reads `text` as under either default layout, if it does. Allocates nothing, so
/// the question can be asked of every string an ordering comparison or an addition meets.
pub(crate) fn yq_datetime(text: &str) -> Option<Instant> {
    let text = text.as_bytes();
    parse_fields(RFC3339.as_bytes(), text)
        .or_else(|_| parse_fields(DATE.as_bytes(), text))
        .ok()
        // Neither layout names a zone, so the instant does not depend on the local zone.
        .map(|fields| fields.resolve(&UtcLocation).nanos)
}

/// The error yq reports for a `text` neither default layout parses (the date layout's, the last
/// one it tries), or `None` when it does parse.
pub(crate) fn yq_datetime_error(text: &str) -> Option<String> {
    if parse_fields(RFC3339.as_bytes(), text.as_bytes()).is_ok() {
        return None;
    }
    parse_fields(DATE.as_bytes(), text.as_bytes())
        .err()
        .map(|error| error.describe())
}

/// yq's `parseDateTime`: `time.Parse(layout, text)`, and for the RFC 3339 layout,
/// `time.Parse("2006-01-02", text)` when that fails, whose error is then the one reported.
pub(crate) fn yq_parse_datetime(
    layout: &str,
    text: &str,
    local: &dyn Location,
) -> Result<GoTime, String> {
    match parse(layout, text, local) {
        Err(_) if layout == RFC3339 => parse(DATE, text, local),
        parsed => parsed,
    }
}

/// [`yq_datetime`] and [`yq_datetime_error`] together, for the tests.
#[cfg(test)]
pub(crate) fn parse_yq_datetime(text: &str) -> Result<Instant, String> {
    yq_datetime(text).ok_or_else(|| yq_datetime_error(text).unwrap_or_default())
}

/// `time.Parse(layout, value)`, with `local` standing for `time.Local`. The error is Go's
/// `ParseError` text.
pub(crate) fn parse(layout: &str, value: &str, local: &dyn Location) -> Result<GoTime, String> {
    parse_fields(layout.as_bytes(), value.as_bytes())
        .map(|fields| fields.resolve(local))
        .map_err(|error| error.describe())
}

// ---------------------------------------------------------------------------------------------
// Layout elements.

/// One element of a layout (Go's `std*` constants). A fractional second carries its digit count
/// and whether its separator is a comma.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Std {
    LongMonth,
    Month,
    NumMonth,
    ZeroMonth,
    LongWeekDay,
    WeekDay,
    Day,
    UnderDay,
    ZeroDay,
    UnderYearDay,
    ZeroYearDay,
    Hour,
    Hour12,
    ZeroHour12,
    Minute,
    ZeroMinute,
    Second,
    ZeroSecond,
    LongYear,
    Year,
    PM,
    Pm,
    Tz,
    Iso8601Tz,
    Iso8601SecondsTz,
    Iso8601ShortTz,
    Iso8601ColonTz,
    Iso8601ColonSecondsTz,
    NumTz,
    NumSecondsTz,
    NumShortTz,
    NumColonTz,
    NumColonSecondsTz,
    FracSecond0 { digits: usize, comma: bool },
    FracSecond9 { digits: usize, comma: bool },
}

impl Std {
    fn is_iso8601(self) -> bool {
        matches!(
            self,
            Self::Iso8601Tz
                | Self::Iso8601SecondsTz
                | Self::Iso8601ShortTz
                | Self::Iso8601ColonTz
                | Self::Iso8601ColonSecondsTz
        )
    }

    fn is_frac(self) -> bool {
        matches!(self, Self::FracSecond0 { .. } | Self::FracSecond9 { .. })
    }
}

fn starts_with_lower_case(s: &[u8]) -> bool {
    s.first().is_some_and(u8::is_ascii_lowercase)
}

fn is_digit(s: &[u8], i: usize) -> bool {
    s.get(i).is_some_and(u8::is_ascii_digit)
}

/// Go's `nextStdChunk`: the text before the first layout element, the element, and the rest.
fn next_std_chunk(layout: &[u8]) -> (&[u8], Option<Std>, &[u8]) {
    let at = move |i: usize, s: &[u8]| layout.get(i..i + s.len()) == Some(s);
    for i in 0..layout.len() {
        let chunk = move |std: Std, len: usize| (&layout[..i], Some(std), &layout[i + len..]);
        match layout[i] {
            b'J' if at(i, b"Jan") => {
                if at(i, b"January") {
                    return chunk(Std::LongMonth, 7);
                }
                if !starts_with_lower_case(&layout[i + 3..]) {
                    return chunk(Std::Month, 3);
                }
            }
            b'M' if layout.len() >= i + 3 => {
                if at(i, b"Mon") {
                    if at(i, b"Monday") {
                        return chunk(Std::LongWeekDay, 6);
                    }
                    if !starts_with_lower_case(&layout[i + 3..]) {
                        return chunk(Std::WeekDay, 3);
                    }
                }
                if at(i, b"MST") {
                    return chunk(Std::Tz, 3);
                }
            }
            b'0' => {
                if let Some(&next @ b'1'..=b'6') = layout.get(i + 1) {
                    let std = [
                        Std::ZeroMonth,
                        Std::ZeroDay,
                        Std::ZeroHour12,
                        Std::ZeroMinute,
                        Std::ZeroSecond,
                        Std::Year,
                    ][usize::from(next - b'1')];
                    return chunk(std, 2);
                }
                if at(i, b"002") {
                    return chunk(Std::ZeroYearDay, 3);
                }
            }
            b'1' => {
                if at(i, b"15") {
                    return chunk(Std::Hour, 2);
                }
                return chunk(Std::NumMonth, 1);
            }
            b'2' => {
                if at(i, b"2006") {
                    return chunk(Std::LongYear, 4);
                }
                return chunk(Std::Day, 1);
            }
            b'_' => {
                if layout.get(i + 1) == Some(&b'2') {
                    // `_2006` is a literal `_` and then the long year.
                    if at(i + 1, b"2006") {
                        return (&layout[..=i], Some(Std::LongYear), &layout[i + 5..]);
                    }
                    return chunk(Std::UnderDay, 2);
                }
                if at(i, b"__2") {
                    return chunk(Std::UnderYearDay, 3);
                }
            }
            b'3' => return chunk(Std::Hour12, 1),
            b'4' => return chunk(Std::Minute, 1),
            b'5' => return chunk(Std::Second, 1),
            b'P' if at(i, b"PM") => return chunk(Std::PM, 2),
            b'p' if at(i, b"pm") => return chunk(Std::Pm, 2),
            b'-' => {
                for (text, std) in [
                    (&b"-070000"[..], Std::NumSecondsTz),
                    (b"-07:00:00", Std::NumColonSecondsTz),
                    (b"-0700", Std::NumTz),
                    (b"-07:00", Std::NumColonTz),
                    (b"-07", Std::NumShortTz),
                ] {
                    if at(i, text) {
                        return chunk(std, text.len());
                    }
                }
            }
            b'Z' => {
                for (text, std) in [
                    (&b"Z070000"[..], Std::Iso8601SecondsTz),
                    (b"Z07:00:00", Std::Iso8601ColonSecondsTz),
                    (b"Z0700", Std::Iso8601Tz),
                    (b"Z07:00", Std::Iso8601ColonTz),
                    (b"Z07", Std::Iso8601ShortTz),
                ] {
                    if at(i, text) {
                        return chunk(std, text.len());
                    }
                }
            }
            // `.000`/`,000` or `.999`/`,999`: a fractional second, if the run of digits ends there.
            sep @ (b'.' | b',') => {
                if let Some(&ch @ (b'0' | b'9')) = layout.get(i + 1) {
                    let mut j = i + 1;
                    while layout.get(j) == Some(&ch) {
                        j += 1;
                    }
                    if !is_digit(layout, j) {
                        let (digits, comma) = (j - (i + 1), sep == b',');
                        let std = if ch == b'0' {
                            Std::FracSecond0 { digits, comma }
                        } else {
                            Std::FracSecond9 { digits, comma }
                        };
                        return (&layout[..i], Some(std), &layout[j..]);
                    }
                }
            }
            _ => {}
        }
    }
    (layout, None, &[])
}

// ---------------------------------------------------------------------------------------------
// Parsing.

const SHORT_DAY_NAMES: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const LONG_DAY_NAMES: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const SHORT_MONTH_NAMES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const LONG_MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Why `time.Parse` failed: what Go's `ParseError` is built from, kept apart from its text so
/// that a probe that only asks whether a string parses allocates nothing.
struct ParseError<'a> {
    layout: &'a [u8],
    value: &'a [u8],
    kind: ParseErrorKind<'a>,
}

enum ParseErrorKind<'a> {
    /// `cannot parse "<value element>" as "<layout element>"`.
    Element {
        layout_elem: &'a [u8],
        value_elem: &'a [u8],
    },
    /// `extra text: "<rest>"`.
    ExtraText(&'a [u8]),
    /// Any other message: `month out of range`, `day-of-year does not match day`.
    Message(&'static str),
}

impl ParseError<'_> {
    /// Go's `ParseError.Error()`.
    fn describe(&self) -> String {
        let value = quote(self.value);
        match &self.kind {
            ParseErrorKind::Element {
                layout_elem,
                value_elem,
            } => format!(
                "parsing time {value} as {}: cannot parse {} as {}",
                quote(self.layout),
                quote(value_elem),
                quote(layout_elem)
            ),
            ParseErrorKind::ExtraText(rest) => {
                format!("parsing time {value}: extra text: {}", quote(rest))
            }
            ParseErrorKind::Message(message) => format!("parsing time {value}: {message}"),
        }
    }
}

/// Go's `time.quote`: a quoted string with `"` and `\` escaped and every byte that is not
/// printable ASCII written as `\xNN`.
fn quote(bytes: &[u8]) -> String {
    let mut out = String::from("\"");
    for &b in bytes {
        if b == b'"' || b == b'\\' {
            out.push('\\');
            out.push(char::from(b));
        } else if !(b' '..0x80).contains(&b) {
            out.push_str(&format!("\\x{b:02x}"));
        } else {
            out.push(char::from(b));
        }
    }
    out.push('"');
    out
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

/// Go's `getnum3`: one to three digits, or exactly three when `fixed`.
fn getnum3(s: &[u8], fixed: bool) -> Option<(i64, &[u8])> {
    let mut n = 0;
    let mut i = 0;
    while i < 3 && is_digit(s, i) {
        n = n * 10 + i64::from(s[i] - b'0');
        i += 1;
    }
    if i == 0 || (fixed && i != 3) {
        return None;
    }
    Some((n, &s[i..]))
}

/// Go's `leadingInt`: the digits at the start of `s`, `None` past `1 << 63`.
fn leading_int(s: &[u8]) -> Option<(u64, &[u8])> {
    let mut x: u64 = 0;
    let mut i = 0;
    while let Some(&c) = s.get(i) {
        if !c.is_ascii_digit() {
            break;
        }
        if x > (1 << 63) / 10 {
            return None;
        }
        x = x * 10 + u64::from(c - b'0');
        if x > 1 << 63 {
            return None;
        }
        i += 1;
    }
    Some((x, &s[i..]))
}

/// Go's `atoi`: an optionally signed decimal integer that is the whole of `s`.
fn atoi(s: &[u8]) -> Option<i64> {
    let (negative, digits) = match s.first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let (q, rest) = leading_int(digits)?;
    if !rest.is_empty() {
        return None;
    }
    // Go converts with `int(q)`, which wraps past `i64::MAX`; no caller reads that many digits.
    let x = q as i64;
    Some(if negative { x.wrapping_neg() } else { x })
}

/// Go's `match`: ASCII letters compared without case.
fn match_fold(s1: &[u8], s2: &[u8]) -> bool {
    s1.len() == s2.len() && s1.iter().zip(s2).all(|(a, b)| a.eq_ignore_ascii_case(b))
}

/// Go's `lookup`: the first name in `table` that `value` starts with, ignoring case.
fn lookup<'a>(table: &[&str], value: &'a [u8]) -> Option<(usize, &'a [u8])> {
    table.iter().enumerate().find_map(|(i, name)| {
        let name = name.as_bytes();
        (value.len() >= name.len() && match_fold(&value[..name.len()], name))
            .then(|| (i, &value[name.len()..]))
    })
}

fn trim_leading_spaces(s: &[u8]) -> &[u8] {
    let n = s.iter().take_while(|&&b| b == b' ').count();
    &s[n..]
}

/// Go's `skip`: `value` with the literal `prefix` removed, where a space in the prefix matches
/// any run of spaces. On a mismatch, the value from where it failed.
fn skip<'a>(mut value: &'a [u8], mut prefix: &[u8]) -> Result<&'a [u8], &'a [u8]> {
    while let Some(&p) = prefix.first() {
        if p == b' ' {
            if value.first().is_some_and(|&b| b != b' ') {
                return Err(value);
            }
            prefix = trim_leading_spaces(prefix);
            value = trim_leading_spaces(value);
            continue;
        }
        if value.first() != Some(&p) {
            return Err(value);
        }
        prefix = &prefix[1..];
        value = &value[1..];
    }
    Ok(value)
}

fn comma_or_period(b: u8) -> bool {
    b == b'.' || b == b','
}

/// Go's `parseNanoseconds`: the fraction in the first `nbytes` bytes of `value`, its separator
/// included. `Err(Some(message))` is a range error, `Err(None)` a parse error.
fn parse_nanoseconds(value: &[u8], nbytes: usize) -> Result<i64, Option<&'static str>> {
    if !value.first().copied().is_some_and(comma_or_period) {
        return Err(None);
    }
    let nbytes = nbytes.min(10);
    let mut ns = atoi(&value[1..nbytes]).ok_or(None)?;
    if ns < 0 {
        return Err(Some("fractional second out of range"));
    }
    for _ in 0..10 - nbytes {
        ns *= 10;
    }
    Ok(ns)
}

/// Go's `parseSignedOffset`: the length of a `+h`/`-hh` offset of at most 12 hours, or 0.
fn parse_signed_offset(value: &[u8]) -> usize {
    if !matches!(value.first(), Some(b'+' | b'-')) {
        return 0;
    }
    match leading_int(&value[1..]) {
        Some((x, rest)) if rest.len() != value.len() - 1 && x <= 12 => value.len() - rest.len(),
        _ => 0,
    }
}

/// Go's `parseTimeZone`: the length of a zone abbreviation at the start of `value`.
fn parse_time_zone(value: &[u8]) -> Option<usize> {
    if value.len() < 3 {
        return None;
    }
    if value.starts_with(b"ChST") || value.starts_with(b"MeST") {
        return Some(4);
    }
    if value.starts_with(b"GMT") {
        // `parseGMT`: an hour offset may follow.
        return Some(3 + parse_signed_offset(&value[3..]));
    }
    if value[0] == b'+' || value[0] == b'-' {
        let n = parse_signed_offset(value);
        return (n > 0).then_some(n);
    }
    let upper = value
        .iter()
        .take(6)
        .take_while(|b| b.is_ascii_uppercase())
        .count();
    match upper {
        5 if value[4] == b'T' => Some(5),
        4 if value[3] == b'T' || value.starts_with(b"WITA") => Some(4),
        3 => Some(3),
        _ => None,
    }
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

/// Days before the first of `month` (1-13) in a common year: Go's `daysBefore`.
fn days_before(month: i64) -> i64 {
    const DAYS_BEFORE: [i64; 14] = [
        0, 0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334, 365,
    ];
    DAYS_BEFORE[month.clamp(0, 13) as usize]
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

/// The year, month (1-12) and day of the date `days` days after 1970-01-01.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// What a successful parse read, before the zone is resolved.
struct Fields<'a> {
    /// Nanoseconds since the epoch of the wall clock read, as if it were UTC.
    wall: Instant,
    /// A `Z` or `UTC` was read.
    utc: bool,
    /// A numeric offset was read, in seconds east of UTC.
    zone_offset: Option<i64>,
    /// A zone abbreviation was read.
    zone_name: Option<&'a [u8]>,
}

impl Fields<'_> {
    /// The end of Go's `parse`: which `Location` the time gets.
    fn resolve(&self, local: &dyn Location) -> GoTime {
        if self.utc {
            return GoTime::new(self.wall, Zone::Utc);
        }
        let name = self
            .zone_name
            .map(|name| String::from_utf8_lossy(name).into_owned());
        if let Some(zone_offset) = self.zone_offset {
            let nanos = self.wall - i128::from(zone_offset) * NANOS_PER_SECOND;
            let unix = nanos.div_euclid(NANOS_PER_SECOND) as i64;
            let (local_name, offset) = local.lookup(unix);
            if offset == zone_offset && name.as_ref().map_or(true, |name| *name == local_name) {
                return GoTime::new(nanos, Zone::Local);
            }
            let name = name.unwrap_or_default();
            return GoTime::new(
                nanos,
                Zone::Fixed {
                    name,
                    offset: zone_offset,
                },
            );
        }
        if let Some(name) = name {
            let wall_unix = self.wall.div_euclid(NANOS_PER_SECOND) as i64;
            if let Some(offset) = local.lookup_name(&name, wall_unix) {
                let nanos = self.wall - i128::from(offset) * NANOS_PER_SECOND;
                return GoTime::new(nanos, Zone::Local);
            }
            // A zone the local one does not know gets offset 0, except `GMT+h`.
            let offset = match name.strip_prefix("GMT") {
                Some(hours) if !hours.is_empty() => {
                    atoi(hours.as_bytes()).unwrap_or(0).saturating_mul(3600)
                }
                _ => 0,
            };
            return GoTime::new(self.wall, Zone::Fixed { name, offset });
        }
        GoTime::new(self.wall, Zone::Utc)
    }
}

/// Go's `parse`, up to the choice of zone.
fn parse_fields<'a>(layout: &'a [u8], value: &'a [u8]) -> Result<Fields<'a>, ParseError<'a>> {
    let (whole_layout, whole_value) = (layout, value);
    let fail = |kind| ParseError {
        layout: whole_layout,
        value: whole_value,
        kind,
    };
    let (mut layout, mut value) = (layout, value);
    let (mut am, mut pm) = (false, false);
    let (mut year, mut month, mut day, mut yday) = (0i64, -1i64, -1i64, -1i64);
    let (mut hour, mut min, mut sec, mut nsec) = (0i64, 0i64, 0i64, 0i64);
    let mut utc = false;
    let mut zone_offset = None;
    let mut zone_name = None;

    loop {
        let (prefix, std, suffix) = next_std_chunk(layout);
        let std_str = &layout[prefix.len()..layout.len() - suffix.len()];
        value = skip(value, prefix).map_err(|at| {
            fail(ParseErrorKind::Element {
                layout_elem: prefix,
                value_elem: at,
            })
        })?;
        let Some(std) = std else {
            if !value.is_empty() {
                return Err(fail(ParseErrorKind::ExtraText(value)));
            }
            break;
        };
        layout = suffix;
        let hold = value;
        let mut range_error: Option<&'static str> = None;
        let ok = match std {
            Std::Year => match value.get(..2).and_then(atoi) {
                Some(y) => {
                    value = &value[2..];
                    year = if y >= 69 { y + 1900 } else { y + 2000 };
                    true
                }
                None => false,
            },
            Std::LongYear => {
                if value.len() < 4 || !is_digit(value, 0) {
                    false
                } else {
                    let digits = &value[..4];
                    value = &value[4..];
                    atoi(digits).map(|y| year = y).is_some()
                }
            }
            Std::Month | Std::LongMonth => {
                let table: &[&str] = if std == Std::Month {
                    &SHORT_MONTH_NAMES
                } else {
                    &LONG_MONTH_NAMES
                };
                match lookup(table, value) {
                    Some((i, rest)) => {
                        month = i as i64 + 1;
                        value = rest;
                        true
                    }
                    None => false,
                }
            }
            Std::NumMonth | Std::ZeroMonth => match getnum(value, std == Std::ZeroMonth) {
                Some((m, rest)) => {
                    month = m;
                    value = rest;
                    if !(1..=12).contains(&m) {
                        range_error = Some("month out of range");
                    }
                    true
                }
                None => false,
            },
            Std::WeekDay | Std::LongWeekDay => {
                let table: &[&str] = if std == Std::WeekDay {
                    &SHORT_DAY_NAMES
                } else {
                    &LONG_DAY_NAMES
                };
                match lookup(table, value) {
                    Some((_, rest)) => {
                        value = rest;
                        true
                    }
                    None => false,
                }
            }
            Std::Day | Std::UnderDay | Std::ZeroDay => {
                if std == Std::UnderDay && value.first() == Some(&b' ') {
                    value = &value[1..];
                }
                // Any one- or two-digit day; the date is checked once the whole layout is read.
                match getnum(value, std == Std::ZeroDay) {
                    Some((d, rest)) => {
                        day = d;
                        value = rest;
                        true
                    }
                    None => false,
                }
            }
            Std::UnderYearDay | Std::ZeroYearDay => {
                for _ in 0..2 {
                    if std == Std::UnderYearDay && value.first() == Some(&b' ') {
                        value = &value[1..];
                    }
                }
                match getnum3(value, std == Std::ZeroYearDay) {
                    Some((d, rest)) => {
                        yday = d;
                        value = rest;
                        true
                    }
                    None => false,
                }
            }
            Std::Hour => match getnum(value, false) {
                Some((h, rest)) => {
                    hour = h;
                    value = rest;
                    if h >= 24 {
                        range_error = Some("hour out of range");
                    }
                    true
                }
                None => false,
            },
            Std::Hour12 | Std::ZeroHour12 => match getnum(value, std == Std::ZeroHour12) {
                Some((h, rest)) => {
                    hour = h;
                    value = rest;
                    if h > 12 {
                        range_error = Some("hour out of range");
                    }
                    true
                }
                None => false,
            },
            Std::Minute | Std::ZeroMinute => match getnum(value, std == Std::ZeroMinute) {
                Some((m, rest)) => {
                    min = m;
                    value = rest;
                    if m >= 60 {
                        range_error = Some("minute out of range");
                    }
                    true
                }
                None => false,
            },
            Std::Second | Std::ZeroSecond => match getnum(value, std == Std::ZeroSecond) {
                Some((s, rest)) => {
                    sec = s;
                    value = rest;
                    if s >= 60 {
                        range_error = Some("second out of range");
                        true
                    } else if value.len() >= 2
                        && comma_or_period(value[0])
                        && is_digit(value, 1)
                        && !next_std_chunk(layout).1.is_some_and(Std::is_frac)
                    {
                        // A fraction after the seconds, though the layout has none.
                        let mut n = 2;
                        while is_digit(value, n) {
                            n += 1;
                        }
                        let parsed = parse_nanoseconds(value, n);
                        value = &value[n..];
                        match parsed {
                            Ok(ns) => {
                                nsec = ns;
                                true
                            }
                            Err(Some(message)) => {
                                range_error = Some(message);
                                true
                            }
                            Err(None) => false,
                        }
                    } else {
                        true
                    }
                }
                None => false,
            },
            Std::PM | Std::Pm => match value.get(..2) {
                Some(p) => {
                    value = &value[2..];
                    let (pm_text, am_text): (&[u8], &[u8]) = if std == Std::PM {
                        (b"PM", b"AM")
                    } else {
                        (b"pm", b"am")
                    };
                    if p == pm_text {
                        pm = true;
                        true
                    } else if p == am_text {
                        am = true;
                        true
                    } else {
                        false
                    }
                }
                None => false,
            },
            Std::Iso8601Tz
            | Std::Iso8601ShortTz
            | Std::Iso8601ColonTz
            | Std::Iso8601SecondsTz
            | Std::Iso8601ColonSecondsTz
            | Std::NumTz
            | Std::NumShortTz
            | Std::NumColonTz
            | Std::NumSecondsTz
            | Std::NumColonSecondsTz => {
                if std.is_iso8601() && value.first() == Some(&b'Z') {
                    value = &value[1..];
                    utc = true;
                    true
                } else {
                    match parse_numeric_zone(std, value) {
                        Some((offset, rest, error)) => {
                            value = rest;
                            range_error = error;
                            match offset {
                                Some(offset) => {
                                    zone_offset = Some(offset);
                                    true
                                }
                                None => false,
                            }
                        }
                        None => false,
                    }
                }
            }
            Std::Tz => {
                if value.starts_with(b"UTC") {
                    utc = true;
                    value = &value[3..];
                    true
                } else {
                    match parse_time_zone(value) {
                        Some(n) => {
                            zone_name = Some(&value[..n]);
                            value = &value[n..];
                            true
                        }
                        None => false,
                    }
                }
            }
            Std::FracSecond0 { digits, .. } => {
                // The exact number of digits the layout has.
                let ndigit = 1 + digits;
                if value.len() < ndigit {
                    false
                } else {
                    let parsed = parse_nanoseconds(value, ndigit);
                    value = &value[ndigit..];
                    match parsed {
                        Ok(ns) => {
                            nsec = ns;
                            true
                        }
                        Err(Some(message)) => {
                            range_error = Some(message);
                            true
                        }
                        Err(None) => false,
                    }
                }
            }
            Std::FracSecond9 { .. } => {
                if value.len() < 2 || !comma_or_period(value[0]) || !is_digit(value, 1) {
                    // The fraction was left out.
                    true
                } else {
                    // Any number of digits, even more than asked for, as after a second.
                    let mut i = 0;
                    while is_digit(value, i + 1) {
                        i += 1;
                    }
                    let parsed = parse_nanoseconds(value, 1 + i);
                    value = &value[1 + i..];
                    match parsed {
                        Ok(ns) => {
                            nsec = ns;
                            true
                        }
                        Err(Some(message)) => {
                            range_error = Some(message);
                            true
                        }
                        Err(None) => false,
                    }
                }
            }
        };
        if let Some(message) = range_error {
            return Err(fail(ParseErrorKind::Message(message)));
        }
        if !ok {
            return Err(fail(ParseErrorKind::Element {
                layout_elem: std_str,
                value_elem: hold,
            }));
        }
    }

    if pm && hour < 12 {
        hour += 12;
    } else if am && hour == 12 {
        hour = 0;
    }

    if yday >= 0 {
        let (mut m, mut d) = (0, 0);
        if is_leap(year) {
            if yday == 31 + 29 {
                m = 2;
                d = 29;
            } else if yday > 31 + 29 {
                yday -= 1;
            }
        }
        if !(1..=365).contains(&yday) {
            return Err(fail(ParseErrorKind::Message("day-of-year out of range")));
        }
        if m == 0 {
            m = (yday - 1) / 31 + 1;
            if days_before(m + 1) < yday {
                m += 1;
            }
            d = yday - days_before(m);
        }
        if month >= 0 && month != m {
            return Err(fail(ParseErrorKind::Message(
                "day-of-year does not match month",
            )));
        }
        month = m;
        if day >= 0 && day != d {
            return Err(fail(ParseErrorKind::Message(
                "day-of-year does not match day",
            )));
        }
        day = d;
    } else {
        if month < 0 {
            month = 1;
        }
        if day < 0 {
            day = 1;
        }
    }

    if day < 1 || day > days_in(month, year) {
        return Err(fail(ParseErrorKind::Message("day out of range")));
    }

    let seconds = days_from_civil(year, month, day) * 86_400 + hour * 3_600 + min * 60 + sec;
    Ok(Fields {
        wall: i128::from(seconds) * NANOS_PER_SECOND + i128::from(nsec),
        utc,
        zone_offset,
        zone_name,
    })
}

/// A numeric zone offset (`-0700`, `-07:00`, `-07`, `-070000`, `-07:00:00` or their `Z`
/// forms after the `Z` check): the offset in seconds (`None` for a bad sign), the rest, and a
/// range error. `None` when the text is too short or not digits.
#[allow(clippy::type_complexity)]
fn parse_numeric_zone(
    std: Std,
    value: &[u8],
) -> Option<(Option<i64>, &[u8], Option<&'static str>)> {
    let (sign, hours, minutes, seconds, rest): (u8, &[u8], &[u8], &[u8], &[u8]) = match std {
        Std::Iso8601ColonTz | Std::NumColonTz => {
            if value.len() < 6 || value[3] != b':' {
                return None;
            }
            (value[0], &value[1..3], &value[4..6], b"00", &value[6..])
        }
        Std::NumShortTz | Std::Iso8601ShortTz => {
            if value.len() < 3 {
                return None;
            }
            (value[0], &value[1..3], b"00", b"00", &value[3..])
        }
        Std::Iso8601ColonSecondsTz | Std::NumColonSecondsTz => {
            if value.len() < 9 || value[3] != b':' || value[6] != b':' {
                return None;
            }
            (
                value[0],
                &value[1..3],
                &value[4..6],
                &value[7..9],
                &value[9..],
            )
        }
        Std::Iso8601SecondsTz | Std::NumSecondsTz => {
            if value.len() < 7 {
                return None;
            }
            (
                value[0],
                &value[1..3],
                &value[3..5],
                &value[5..7],
                &value[7..],
            )
        }
        _ => {
            if value.len() < 5 {
                return None;
            }
            (value[0], &value[1..3], &value[3..5], b"00", &value[5..])
        }
    };
    let hr = getnum(hours, true)?.0;
    let mm = getnum(minutes, true)?.0;
    let ss = getnum(seconds, true)?.0;
    // `>` rather than `>=`: some people write offsets of 24 hours or 60 minutes or seconds.
    // Go assigns the three in turn, so the last one out of range is the one reported.
    let range_error = if ss > 60 {
        Some("time zone offset second out of range")
    } else if mm > 60 {
        Some("time zone offset minute out of range")
    } else if hr > 24 {
        Some("time zone offset hour out of range")
    } else {
        None
    };
    let offset = (hr * 60 + mm) * 60 + ss;
    let offset = match sign {
        b'+' => Some(offset),
        b'-' => Some(-offset),
        _ => None,
    };
    Some((offset, rest, range_error))
}

// ---------------------------------------------------------------------------------------------
// Formatting.

/// Go's `appendInt`: `x` in decimal, zero-padded to `width` digits after any sign.
fn push_int(out: &mut String, x: i64, width: usize) {
    if x < 0 {
        out.push('-');
    }
    out.push_str(&format!("{:0width$}", x.unsigned_abs()));
}

/// `t.Format(layout)`, with `local` standing for `time.Local`.
pub(crate) fn format(time: &GoTime, layout: &str, local: &dyn Location) -> String {
    let (name, offset) = time.zone_at(local);
    let local_nanos = time.nanos + i128::from(offset) * NANOS_PER_SECOND;
    let local_secs = local_nanos.div_euclid(NANOS_PER_SECOND) as i64;
    let nanosecond = local_nanos.rem_euclid(NANOS_PER_SECOND) as i64;
    let days = local_secs.div_euclid(86_400);
    let second_of_day = local_secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let yday = days - days_from_civil(year, 1, 1) + 1;
    let weekday = (days + 4).rem_euclid(7) as usize;
    let (hour, min, sec) = (
        second_of_day / 3_600,
        second_of_day % 3_600 / 60,
        second_of_day % 60,
    );

    let mut out = String::new();
    let mut layout = layout.as_bytes();
    loop {
        let (prefix, std, suffix) = next_std_chunk(layout);
        out.push_str(&String::from_utf8_lossy(prefix));
        let Some(std) = std else {
            break;
        };
        layout = suffix;
        let hour12 = if hour % 12 == 0 { 12 } else { hour % 12 };
        match std {
            Std::Year => push_int(&mut out, year.abs() % 100, 2),
            Std::LongYear => push_int(&mut out, year, 4),
            Std::Month => out.push_str(SHORT_MONTH_NAMES[(month - 1) as usize]),
            Std::LongMonth => out.push_str(LONG_MONTH_NAMES[(month - 1) as usize]),
            Std::NumMonth => push_int(&mut out, month, 0),
            Std::ZeroMonth => push_int(&mut out, month, 2),
            Std::WeekDay => out.push_str(SHORT_DAY_NAMES[weekday]),
            Std::LongWeekDay => out.push_str(LONG_DAY_NAMES[weekday]),
            Std::Day => push_int(&mut out, day, 0),
            Std::UnderDay => {
                if day < 10 {
                    out.push(' ');
                }
                push_int(&mut out, day, 0);
            }
            Std::ZeroDay => push_int(&mut out, day, 2),
            Std::UnderYearDay => {
                if yday < 100 {
                    out.push(' ');
                    if yday < 10 {
                        out.push(' ');
                    }
                }
                push_int(&mut out, yday, 0);
            }
            Std::ZeroYearDay => push_int(&mut out, yday, 3),
            Std::Hour => push_int(&mut out, hour, 2),
            Std::Hour12 => push_int(&mut out, hour12, 0),
            Std::ZeroHour12 => push_int(&mut out, hour12, 2),
            Std::Minute => push_int(&mut out, min, 0),
            Std::ZeroMinute => push_int(&mut out, min, 2),
            Std::Second => push_int(&mut out, sec, 0),
            Std::ZeroSecond => push_int(&mut out, sec, 2),
            Std::PM => out.push_str(if hour >= 12 { "PM" } else { "AM" }),
            Std::Pm => out.push_str(if hour >= 12 { "pm" } else { "am" }),
            Std::Iso8601Tz
            | Std::Iso8601ColonTz
            | Std::Iso8601SecondsTz
            | Std::Iso8601ShortTz
            | Std::Iso8601ColonSecondsTz
            | Std::NumTz
            | Std::NumColonTz
            | Std::NumSecondsTz
            | Std::NumShortTz
            | Std::NumColonSecondsTz => {
                // The `Z` forms write `Z` for UTC, ISO 8601's way.
                if offset == 0 && std.is_iso8601() {
                    out.push('Z');
                    continue;
                }
                let zone = offset / 60;
                out.push(if zone < 0 { '-' } else { '+' });
                let (zone, abs_offset) = (zone.abs(), offset.abs());
                push_int(&mut out, zone / 60, 2);
                if matches!(
                    std,
                    Std::Iso8601ColonTz
                        | Std::NumColonTz
                        | Std::Iso8601ColonSecondsTz
                        | Std::NumColonSecondsTz
                ) {
                    out.push(':');
                }
                if !matches!(std, Std::NumShortTz | Std::Iso8601ShortTz) {
                    push_int(&mut out, zone % 60, 2);
                }
                if matches!(
                    std,
                    Std::Iso8601SecondsTz
                        | Std::NumSecondsTz
                        | Std::NumColonSecondsTz
                        | Std::Iso8601ColonSecondsTz
                ) {
                    if matches!(std, Std::NumColonSecondsTz | Std::Iso8601ColonSecondsTz) {
                        out.push(':');
                    }
                    push_int(&mut out, abs_offset % 60, 2);
                }
            }
            Std::Tz => {
                if !name.is_empty() {
                    out.push_str(&name);
                    continue;
                }
                // No zone name is known, but one must be written: the `-0700` form.
                let zone = offset / 60;
                out.push(if zone < 0 { '-' } else { '+' });
                let zone = zone.abs();
                push_int(&mut out, zone / 60, 2);
                push_int(&mut out, zone % 60, 2);
            }
            Std::FracSecond0 { digits, comma } | Std::FracSecond9 { digits, comma } => {
                let trim = matches!(std, Std::FracSecond9 { .. });
                if trim && (digits == 0 || nanosecond == 0) {
                    continue;
                }
                let dot = if comma { ',' } else { '.' };
                let nine = format!("{nanosecond:09}");
                let mut fraction = String::from(&nine[..digits.min(9)]);
                if trim {
                    let kept = fraction.trim_end_matches('0').len();
                    fraction.truncate(kept);
                    if fraction.is_empty() {
                        continue;
                    }
                }
                out.push(dot);
                out.push_str(&fraction);
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Durations.

/// Go's `leadingFraction`: the digits at the start of `s` as a fraction `x / scale`, digits past
/// what fits ignored.
fn leading_fraction(s: &[u8]) -> (u64, f64, &[u8]) {
    let mut x: u64 = 0;
    let mut scale = 1.0;
    let mut overflow = false;
    let mut i = 0;
    while let Some(&c) = s.get(i) {
        if !c.is_ascii_digit() {
            break;
        }
        i += 1;
        if overflow {
            continue;
        }
        if x > (i64::MAX as u64) / 10 {
            overflow = true;
            continue;
        }
        let y = x * 10 + u64::from(c - b'0');
        if y > 1 << 63 {
            overflow = true;
            continue;
        }
        x = y;
        scale *= 10.0;
    }
    (x, scale, &s[i..])
}

/// `time.ParseDuration(text)`: a signed sequence of decimal numbers, each with an optional
/// fraction and a unit (`ns`, `us`/`µs`/`μs`, `ms`, `s`, `m`, `h`), in nanoseconds. The error is
/// Go's text.
pub(crate) fn parse_duration(text: &str) -> Result<i64, String> {
    let orig = text.as_bytes();
    let invalid = || format!("time: invalid duration {}", quote(orig));
    let mut s = orig;
    let mut negative = false;
    if let Some(&c @ (b'-' | b'+')) = s.first() {
        negative = c == b'-';
        s = &s[1..];
    }
    if s == b"0" {
        return Ok(0);
    }
    if s.is_empty() {
        return Err(invalid());
    }
    let mut d: u64 = 0;
    while !s.is_empty() {
        if !(s[0] == b'.' || s[0].is_ascii_digit()) {
            return Err(invalid());
        }
        let before = s.len();
        let (v, rest) = leading_int(s).ok_or_else(invalid)?;
        s = rest;
        let pre = before != s.len();
        let (mut f, mut scale, mut post) = (0, 1.0, false);
        if s.first() == Some(&b'.') {
            s = &s[1..];
            let before = s.len();
            let (fx, fscale, rest) = leading_fraction(s);
            (f, scale, s) = (fx, fscale, rest);
            post = before != s.len();
        }
        if !pre && !post {
            return Err(invalid());
        }
        let unit_len = s
            .iter()
            .take_while(|&&c| c != b'.' && !c.is_ascii_digit())
            .count();
        if unit_len == 0 {
            return Err(format!("time: missing unit in duration {}", quote(orig)));
        }
        let (unit_text, rest) = s.split_at(unit_len);
        s = rest;
        let unit: u64 = match unit_text {
            b"ns" => 1,
            // `µs` (U+00B5, the micro sign) and `μs` (U+03BC, the Greek mu).
            b"us" | b"\xc2\xb5s" | b"\xce\xbcs" => 1_000,
            b"ms" => 1_000_000,
            b"s" => 1_000_000_000,
            b"m" => 60_000_000_000,
            b"h" => 3_600_000_000_000,
            _ => {
                return Err(format!(
                    "time: unknown unit {} in duration {}",
                    quote(unit_text),
                    quote(orig)
                ))
            }
        };
        if v > (1 << 63) / unit {
            return Err(invalid());
        }
        let mut v = v * unit;
        if f > 0 {
            // A float keeps a fraction of an hour accurate to the nanosecond.
            v += (f as f64 * (unit as f64 / scale)) as u64;
            if v > 1 << 63 {
                return Err(invalid());
            }
        }
        d += v;
        if d > 1 << 63 {
            return Err(invalid());
        }
    }
    if negative {
        // `-Duration(d)`, where `d` may be exactly `1 << 63`.
        return Ok((d as i64).wrapping_neg());
    }
    if d > i64::MAX as u64 {
        return Err(invalid());
    }
    Ok(d as i64)
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

    /// A local zone fixed at `offset` seconds, named `name`.
    struct FixedLocal(&'static str, i64);

    impl Location for FixedLocal {
        fn lookup(&self, _unix: i64) -> (String, i64) {
            (String::from(self.0), self.1)
        }

        fn lookup_name(&self, name: &str, _unix: i64) -> Option<i64> {
            (name == self.0).then_some(self.1)
        }
    }

    /// `time.Parse(layout, value)` then `Format(out)`, with the local zone UTC.
    fn reformat(layout: &str, value: &str, out: &str) -> String {
        let t = parse(layout, value, &UtcLocation).unwrap_or_else(|e| panic!("{value}: {e}"));
        format(&t, out, &UtcLocation)
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

    /// `.t | format_datetime(...)` on `t: 2026-01-01T00:00:00Z` and `n: ...+05:30`, captured
    /// from yq v4.53.3.
    #[test]
    fn every_layout_element_formats_as_go_does_5863() {
        let t = "2026-01-01T00:00:00Z";
        assert_eq!(
            reformat(
                RFC3339,
                t,
                "Mon Monday Jan January 1 01 2 _2 02 __2 002 15 3 03 4 04 5 05 06 2006 PM pm \
                 MST Z07 Z0700 Z070000 Z07:00:00 -07 -070000 -07:00:00 .0 ,9"
            ),
            "Thu Thursday Jan January 1 01 1  1 01   1 001 00 12 12 0 00 0 00 26 2026 AM am \
             UTC Z Z Z Z +00 +000000 +00:00:00 .0 "
        );
        assert_eq!(
            reformat(RFC3339, t, "Monday, 02-Jan-06 at 3:04PM MST"),
            "Thursday, 01-Jan-26 at 12:00AM UTC"
        );
        let n = "2026-01-01T10:20:30.5+05:30";
        assert_eq!(
            reformat(
                RFC3339,
                n,
                "Monday, 02-Jan-06 at 3:04PM MST -0700 Z07:00 .000 .999"
            ),
            "Thursday, 01-Jan-26 at 10:20AM +0530 +0530 +05:30 .500 .5"
        );
        assert_eq!(
            reformat(RFC3339, n, "3PM 03pm _2006 Janx Monx"),
            "10AM 10am _2026 Janx Monx"
        );
        let f = "2026-01-01T00:00:00.123456789Z";
        assert_eq!(
            reformat(RFC3339, f, ".000000 .999999999 .9 ,000"),
            ".123456 .123456789 .1 ,123"
        );
        // RFC 3339 drops the fraction and writes `Z` for any zero offset.
        assert_eq!(reformat(RFC3339, f, RFC3339), "2026-01-01T00:00:00Z");
        assert_eq!(
            reformat(RFC3339, "2026-01-01T00:00:00+00:00", RFC3339),
            "2026-01-01T00:00:00Z"
        );
        // An offset of zero is the local zone when that is UTC, and otherwise has no name, so
        // `MST` writes `+0000` (yq printed `0`, the snippet `+0000` read back).
        assert_eq!(reformat(RFC3339, "2026-01-01T00:00:00+00:00", "MST"), "UTC");
        let p = parse(
            RFC3339,
            "2026-01-01T00:00:00+00:00",
            &FixedLocal("AEDT", 39600),
        )
        .unwrap();
        assert_eq!(format(&p, "MST", &FixedLocal("AEDT", 39600)), "+0000");
    }

    #[test]
    fn an_offset_the_local_zone_has_takes_its_name_5863() {
        let local = FixedLocal("AEDT", 11 * 3600);
        let t = parse(RFC3339, "2026-03-29T01:30:00+11:00", &local).unwrap();
        assert_eq!(format(&t, "MST", &local), "AEDT");
        let t = parse(RFC3339, "2026-03-29T01:30:00+10:00", &local).unwrap();
        assert_eq!(format(&t, "MST -0700", &local), "+1000 +1000");
        // A name the local zone has is read with its offset; another gets offset 0, and `GMT+h`
        // is shown at its hours without moving the wall clock off UTC (as yq v4.53.3 prints it).
        let t = parse("2006-01-02 15:04 MST", "2026-01-01 11:00 AEDT", &local).unwrap();
        assert_eq!(format(&t, RFC3339, &UtcLocation), "2026-01-01T00:00:00Z");
        let t = parse("2006-01-02 15:04 MST", "2026-01-01 11:00 PST", &local).unwrap();
        assert_eq!(format(&t, "15:04 MST -0700", &local), "11:00 PST +0000");
        let t = parse("2006-01-02 15:04 MST", "2026-01-01 11:00 GMT+3", &local).unwrap();
        assert_eq!(format(&t, "15:04 MST -0700", &local), "14:00 GMT+3 +0300");
    }

    #[test]
    fn a_custom_layout_reads_names_without_case_and_am_pm_with_it_5863() {
        assert_eq!(reformat("2006", "2001", RFC3339), "2001-01-01T00:00:00Z");
        assert_eq!(
            reformat("Jan 2 2006 3:04pm", "FEB 3 2001 4:05pm", RFC3339),
            "2001-02-03T16:05:00Z"
        );
        assert_eq!(
            parse("3:04pm", "4:05PM", &UtcLocation).unwrap_err(),
            r#"parsing time "4:05PM" as "3:04pm": cannot parse "PM" as "pm""#
        );
    }

    #[test]
    fn custom_layout_errors_match_go_5863() {
        let e = |layout: &str, value: &str| parse(layout, value, &UtcLocation).unwrap_err();
        assert_eq!(
            e("2006", "2026-01-01T00:00:00Z"),
            r#"parsing time "2026-01-01T00:00:00Z": extra text: "-01-01T00:00:00Z""#
        );
        assert_eq!(
            e("Jan 2006", "2026-01-01"),
            r#"parsing time "2026-01-01" as "Jan 2006": cannot parse "2026-01-01" as "Jan""#
        );
        assert_eq!(
            e("1", "2026-01-01T00:00:00Z"),
            r#"parsing time "2026-01-01T00:00:00Z": month out of range"#
        );
        assert_eq!(
            e(".a", "2026-01-01T00:00:00Z"),
            r#"parsing time "2026-01-01T00:00:00Z" as ".a": cannot parse "2026-01-01T00:00:00Z" as ".a""#
        );
        assert_eq!(
            e("2006-01-02", "2026-01-01T00:00:00Z"),
            r#"parsing time "2026-01-01T00:00:00Z": extra text: "T00:00:00Z""#
        );
        assert_eq!(
            e("15:04", "25:00"),
            r#"parsing time "25:00": hour out of range"#
        );
        assert_eq!(
            e("2006 002", "2001 366"),
            r#"parsing time "2001 366": day-of-year out of range"#
        );
    }

    #[test]
    fn a_day_of_year_and_twelve_hour_clock_read_as_go_reads_them_5863() {
        assert_eq!(
            reformat("2006 002", "2024 060", RFC3339),
            "2024-02-29T00:00:00Z"
        );
        assert_eq!(
            reformat("2006 __2", "2023  60", RFC3339),
            "2023-03-01T00:00:00Z"
        );
        assert_eq!(
            reformat("2006 3PM", "2001 12AM", RFC3339),
            "2001-01-01T00:00:00Z"
        );
        assert_eq!(
            reformat("2006 3PM", "2001 12PM", RFC3339),
            "2001-01-01T12:00:00Z"
        );
        assert_eq!(
            reformat("06 Jan _2", "70 MAR  5", RFC3339),
            "1970-03-05T00:00:00Z"
        );
    }

    /// `.t += "<d>"` on `t: 2026-01-01T00:00:00Z`, and the errors, captured from yq v4.53.3.
    #[test]
    fn durations_parse_as_go_parses_them_5863() {
        let h = 3_600_000_000_000i64;
        for (text, want) in [
            ("3h10m", 3 * h + 10 * 60_000_000_000),
            ("24h", 24 * h),
            ("-1h", -h),
            ("+1h", h),
            ("1.5h", h + h / 2),
            ("1ns", 1),
            ("1us", 1_000),
            ("1µs", 1_000),
            ("1μs", 1_000),
            ("1ms", 1_000_000),
            ("0", 0),
            ("-0", 0),
            (".5s", 500_000_000),
            ("1h1h", 2 * h),
            ("2562047h", 2_562_047 * h),
        ] {
            assert_eq!(parse_duration(text), Ok(want), "{text}");
        }
        for (text, want) in [
            ("bogus", r#"time: invalid duration "bogus""#),
            ("-bogus", r#"time: invalid duration "-bogus""#),
            ("", r#"time: invalid duration """#),
            ("null", r#"time: invalid duration "null""#),
            ("1", r#"time: missing unit in duration "1""#),
            ("-1.5", r#"time: missing unit in duration "-1.5""#),
            (".s", r#"time: invalid duration ".s""#),
            (
                "-2026-01-01T00:00:00Z",
                r#"time: unknown unit "-" in duration "-2026-01-01T00:00:00Z""#,
            ),
            ("1d", r#"time: unknown unit "d" in duration "1d""#),
            ("9999999h", r#"time: invalid duration "9999999h""#),
        ] {
            assert_eq!(parse_duration(text), Err(want.to_string()), "{text}");
        }
    }

    #[test]
    fn adding_keeps_the_zone_and_drops_the_fraction_5863() {
        let local = FixedLocal("AEDT", 11 * 3600);
        let add = |text: &str, d: &str| {
            let t = yq_parse_datetime(RFC3339, text, &local).unwrap();
            format(&t.add(parse_duration(d).unwrap()), RFC3339, &local)
        };
        assert_eq!(add("2026-01-01T00:00:00Z", "3h10m"), "2026-01-01T03:10:00Z");
        assert_eq!(add("2026-01-01T00:00:00Z", "-24h"), "2025-12-31T00:00:00Z");
        assert_eq!(add("2026-01-01", "25h"), "2026-01-02T01:00:00Z");
        assert_eq!(
            add("2026-01-01T10:20:30.5+05:30", "1h"),
            "2026-01-01T11:20:30+05:30"
        );
    }
}
