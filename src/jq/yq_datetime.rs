//! yq's datetime operators (#5863): a duration added to or subtracted from a timestamp,
//! `format_datetime`, `tz`, `with_dtf`, and the text `now`, `from_unix` and `to_unix` read and
//! write, over [`super::go_time`]'s port of Go's `time` package.
//!
//! yq (`pkg/yqlib/operator_datetime.go`, `operator_add.go`, `operator_subtract.go`, v4.53.3)
//! keeps a timestamp as text tagged `!!timestamp`. It reads that text with `time.Parse` under the
//! current layout -- RFC 3339 by default, with `2006-01-02` tried when RFC 3339 fails, or whatever
//! `with_dtf` set -- and writes a result back with `Format(layout)`.
//!
//! `OwnedValue` has no timestamp kind, so the text stands in for the tag, as it already does for
//! the ordering comparison (#5651): a string is a timestamp when it parses as RFC 3339 or
//! `2006-01-02`. yq also treats a `!!str` as a timestamp when it parses under a custom layout,
//! and that rule is applied as it is. A *quoted* string spelled like a timestamp is therefore
//! one here and not in yq, which `docs/compliance/yq/limitations.md` records.
//!
//! The local zone is [`super::local_zone`]'s, as `time.Local` is Go's.

use alloc::borrow::Cow;
use alloc::format;
use alloc::string::String;

use super::go_time::{self, GoTime, Instant, Zone, RFC3339};
use super::local_zone::{self, GoLocal};

/// The layout `with_dtf` sets, for everything its expression evaluates (yq's
/// `Context.SetDateTimeLayout`, which every operator below it inherits).
#[cfg(feature = "std")]
mod context {
    use alloc::string::String;
    use core::cell::RefCell;
    use core::sync::atomic::{AtomicBool, Ordering};

    std::thread_local! {
        /// `None` is RFC 3339.
        static LAYOUT: RefCell<Option<String>> = const { RefCell::new(None) };
    }

    /// Whether any thread has ever entered a custom layout. It only goes from `false` to `true`,
    /// and does so before the layout is visible to its own thread, so `false` means this thread's
    /// layout is RFC 3339 and a string `+` can skip the thread-local read (`users_yq_raw_concat`,
    /// every row of which asks).
    static EVER_CUSTOM: AtomicBool = AtomicBool::new(false);

    pub(super) fn ever_custom() -> bool {
        EVER_CUSTOM.load(Ordering::Relaxed)
    }

    pub(super) fn custom() -> Option<String> {
        LAYOUT.with(|layout| layout.borrow().clone())
    }

    /// Restores the layout that was in effect when it was made.
    pub(super) struct Scope(Option<String>);

    pub(super) fn enter(layout: Option<String>) -> Scope {
        if layout.is_some() {
            EVER_CUSTOM.store(true, Ordering::Relaxed);
        }
        Scope(LAYOUT.with(|current| current.replace(layout)))
    }

    impl Drop for Scope {
        fn drop(&mut self) {
            let previous = self.0.take();
            LAYOUT.with(|current| *current.borrow_mut() = previous);
        }
    }
}

/// `no_std` has no `thread_local!`, so no layout but RFC 3339 is ever in effect, and `with_dtf`
/// refuses.
#[cfg(not(feature = "std"))]
mod context {
    use alloc::string::String;

    pub(super) fn custom() -> Option<String> {
        None
    }

    pub(super) fn ever_custom() -> bool {
        false
    }
}

/// The custom layout in effect, `None` for RFC 3339.
fn custom_layout() -> Option<String> {
    context::custom()
}

/// The layout in effect.
fn layout() -> Cow<'static, str> {
    custom_layout().map_or(Cow::Borrowed(RFC3339), Cow::Owned)
}

/// `with_dtf(layout; f)`: `f` evaluated with `layout` as the datetime layout, and the previous
/// one restored afterwards, however `f` ends. `f` must finish its work inside: nothing it returns
/// may evaluate later. The RFC 3339 layout spelled out is the default one, which is the only
/// layout yq also reads `2006-01-02` under, and so is an empty one (yq's `GetDateTimeLayout`
/// answers RFC 3339 for it, as for a container's empty text).
#[cfg(feature = "std")]
pub(crate) fn with_layout<T>(layout: String, f: impl FnOnce() -> T) -> T {
    let custom = (!layout.is_empty() && layout != RFC3339).then_some(layout);
    let _scope = context::enter(custom);
    f()
}

/// Whether the current layout reads `text` when the default ones do not, and the layout if so.
fn custom_datetime(text: &str) -> Option<String> {
    let layout = custom_layout()?;
    go_time::parse(&layout, text, &GoLocal)
        .is_ok()
        .then_some(layout)
}

/// Whether `text` is worth asking [`is_timestamp`] about: under the default layouts a timestamp
/// starts with a four-digit year and a `-` (`2006-01-02`, at least ten bytes), and only a custom
/// layout, which can start with anything, widens that. A hot path (a sort of strings) asks this
/// first to skip a parse that cannot succeed.
pub(crate) fn could_be_timestamp(text: &str) -> bool {
    let bytes = text.as_bytes();
    context::ever_custom()
        || (bytes.len() >= 10 && bytes[..4].iter().all(u8::is_ascii_digit) && bytes[4] == b'-')
}

/// The instant `text` reads as, under the layouts [`compare_instants`] reads it by, for a caller
/// that compares one text many times (a sort) and parses it once.
pub(crate) fn instant_of(text: &str) -> Option<Instant> {
    let layout = match go_time::yq_datetime(text) {
        Some(instant) => match custom_layout() {
            None => return Some(instant),
            Some(layout) => layout,
        },
        None => custom_datetime(text)?,
    };
    go_time::yq_parse_datetime(&layout, text, &GoLocal)
        .ok()
        .map(|time| time.instant())
}

/// Whether yq's `+`, `-` and ordering comparisons treat `text` as a timestamp: it reads under
/// RFC 3339 or `2006-01-02` (standing for the `!!timestamp` tag), or under the custom layout.
pub(crate) fn is_timestamp(text: &str) -> bool {
    go_time::yq_datetime(text).is_some() || custom_datetime(text).is_some()
}

/// Whether `text` can begin a Go duration: after an optional sign, a digit or a `.`, which is
/// where `time.ParseDuration` starts reading a number.
fn starts_like_a_duration(text: &str) -> bool {
    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
    unsigned
        .as_bytes()
        .first()
        .is_some_and(|&b| b.is_ascii_digit() || b == b'.')
}

/// yq's `+` (or `-`, when `subtract`) of a duration to `left`, or `None` when `left` is not a
/// timestamp and the operator is not a date operation. `right` is the right operand's text.
///
/// The duration is read first, then the time: `.t + "bogus1h"` is the duration's error whatever
/// `.t` holds. `-` negates the text before reading it, so its error names `"-bogus"`; the
/// bracketed text is the operand as written.
///
/// The text stands in for yq's `!!timestamp` tag, which a quoted or JSON string does not have:
/// there `"2026-01-01" + "T00:00:00Z"` concatenates. So `+` of text that cannot begin a duration
/// (`T00:00:00Z`, `bogus`, an empty string) to a string that is a timestamp only by its shape
/// concatenates too, as for a string; text that starts like one (`3hrs`, `1`) is read and
/// refused. A string that reads under a custom layout is a timestamp to yq whatever its tag, so
/// that rule has no exception.
pub(crate) fn add_duration(
    left: &str,
    right: &str,
    subtract: bool,
) -> Option<Result<String, String>> {
    // The common case, a string `+` text that cannot begin a duration, is a concatenation unless
    // a custom layout reads `left`; with none ever set it needs neither a layout lookup nor a
    // `time.Parse` of every left side.
    if !subtract && !context::ever_custom() && !starts_like_a_duration(right) {
        return None;
    }
    let layout = match custom_datetime(left) {
        Some(custom) => Cow::Owned(custom),
        None if !subtract && !starts_like_a_duration(right) => return None,
        None if go_time::yq_datetime(left).is_some() => layout(),
        None => return None,
    };
    let duration_text: Cow<'_, str> = match (subtract, right.strip_prefix('-')) {
        (false, _) => Cow::Borrowed(right),
        (true, Some(positive)) => Cow::Borrowed(positive),
        (true, None) => Cow::Owned(format!("-{right}")),
    };
    Some((|| {
        let duration = go_time::parse_duration(&duration_text)
            .map_err(|e| format!("unable to parse duration [{right}]: {e}"))?;
        let time = go_time::yq_parse_datetime(&layout, left, &GoLocal)?;
        Ok(go_time::format(&time.add(duration), &layout, &GoLocal))
    })())
}

/// yq's ordering comparison when `left` is a timestamp: both instants, or the error parsing
/// either raises. `None` when `left` is not one, and the strings compare as strings.
///
/// Under the default layout this allocates nothing unless the right operand fails, so it can be
/// asked of every string comparison.
pub(crate) fn compare_instants(
    left: &str,
    right: &str,
) -> Option<Result<(Instant, Instant), String>> {
    let layout = match go_time::yq_datetime(left) {
        Some(left_instant) => match custom_layout() {
            None => {
                return Some(match go_time::yq_datetime(right) {
                    Some(right_instant) => Ok((left_instant, right_instant)),
                    None => Err(go_time::yq_datetime_error(right).unwrap_or_default()),
                })
            }
            Some(layout) => layout,
        },
        None => custom_datetime(left)?,
    };
    let parse = |text| go_time::yq_parse_datetime(&layout, text, &GoLocal);
    Some(parse(left).and_then(|l| Ok((l.instant(), parse(right)?.instant()))))
}

/// yq's `format_datetime(format)`: `text` read under the layout, written with `format`. The
/// caller reads the result back as a YAML snippet, as yq does.
///
/// yq names the node's path in the error (`[a.b]`); a value here carries no path, so it is
/// always the empty one a computed value has in yq.
pub(crate) fn format_datetime(text: &str, format: &str) -> Result<String, String> {
    let layout = layout();
    let time = go_time::yq_parse_datetime(&layout, text, &GoLocal)
        .map_err(|e| format!("could not parse datetime of []: {e}"))?;
    Ok(go_time::format(&time, format, &GoLocal))
}

/// The error yq wraps a layout's parse failure in for `tz` and `to_unix`.
fn parse_error_using_layout(layout: &str, error: &str) -> String {
    format!("could not parse datetime of [] using layout [{layout}]: {error}")
}

/// What Go's `time.LoadLocation` loaded.
enum Loaded {
    Utc,
    Local,
    Named,
}

/// `time.LoadLocation(name)`: `""` and `UTC` are UTC, `Local` the local zone, and anything else a
/// name in the zone database, Go's error otherwise.
fn load_location(name: &str) -> Result<Loaded, String> {
    match name {
        "" | "UTC" => return Ok(Loaded::Utc),
        "Local" => return Ok(Loaded::Local),
        _ => {}
    }
    if name.contains("..") || name.starts_with('/') || name.starts_with('\\') {
        return Err(String::from("time: invalid location name"));
    }
    match local_zone::named_at(name, 0) {
        Some(_) => Ok(Loaded::Named),
        None => Err(format!("unknown time zone {name}")),
    }
}

/// yq's `tz(zone)`: `text` read under the layout, moved to `zone` and written under the layout.
/// The zone is loaded before the text is read, so a bad zone is the error.
pub(crate) fn tz(text: &str, zone: &str) -> Result<String, String> {
    let loaded = load_location(zone).map_err(|e| format!("could not load tz [{zone}]: {e}"))?;
    let layout = layout();
    let time = go_time::yq_parse_datetime(&layout, text, &GoLocal)
        .map_err(|e| parse_error_using_layout(&layout, &e))?;
    let moved = match loaded {
        Loaded::Utc => time.in_zone(Zone::Utc),
        Loaded::Local => time.in_zone(Zone::Local),
        Loaded::Named => match local_zone::named_at(zone, time.unix()) {
            Some(named) => time.in_zone(Zone::Fixed {
                name: named.name,
                offset: named.offset_secs,
            }),
            None => {
                return Err(format!(
                    "could not load tz [{zone}]: unknown time zone {zone}"
                ))
            } // patchcov: coverage tolerate-line reason="unreachable: load_location found the zone a moment earlier"
        },
    };
    Ok(go_time::format(&moved, &layout, &GoLocal))
}

/// yq's `to_unix`: `text` read under the layout, as whole seconds since the epoch.
pub(crate) fn to_unix(text: &str) -> Result<i64, String> {
    let layout = layout();
    go_time::yq_parse_datetime(&layout, text, &GoLocal)
        .map(|time| time.unix())
        .map_err(|e| parse_error_using_layout(&layout, &e))
}

/// yq's `from_unix`: `time.Unix` of the number's whole and fractional parts (`math.Modf`), in
/// the local zone, as RFC 3339 -- which has no fraction. The layout does not apply.
pub(crate) fn from_unix(seconds: f64) -> String {
    let whole = seconds.trunc();
    // Go's conversions saturate the same way past `i64`.
    let nanos = i128::from(whole as i64) * 1_000_000_000 + ((seconds - whole) * 1e9) as i128;
    go_time::format(&GoTime::new(nanos, Zone::Local), RFC3339, &GoLocal)
}

/// yq's `now`: the time `nanos` after the epoch, in the local zone, as RFC 3339.
pub(crate) fn now(nanos: i128) -> String {
    go_time::format(&GoTime::new(nanos, Zone::Local), RFC3339, &GoLocal)
}
