//! The process locale behind `strftime`/`strflocaltime`'s localized specifiers (#5754).
//!
//! jq calls `setlocale(LC_ALL, "")`, so the weekday and month names, `%p`, and the
//! `%c`/`%x`/`%X`/`%r` layouts follow `LC_ALL`, `LC_TIME` and `LANG`. This module
//! resolves that locale once and renders those specifiers through the C library's
//! `strftime_l`, so the names and layouts are the platform's own -- the pinned oracle's
//! `nl_langinfo` disagrees with its own `strftime` on macOS (`%p`/`%r` print empty under
//! `de_DE`), and only calling `strftime_l` reproduces both.
//!
//! - [`is_active`] -- whether a non-C locale is in effect. The C locale (an unset or `C`
//!   or `POSIX` environment, or a locale that is not installed) answers `false`, and
//!   the evaluator's own English tables then run unchanged.
//! - [`render`] -- one localized conversion specifier for a broken-down time.
//!
//! The locale is resolved with `newlocale(LC_ALL_MASK, "", ..)`, which reads the
//! environment like `setlocale(LC_ALL, "")` but leaves process-global state alone, and
//! applies jq's all-or-nothing rule: if any category is invalid it fails, and the whole
//! process formats as C (`LANG=de_DE.UTF-8 LC_MESSAGES=xx_XX` prints `Thursday`).
//!
//! Without `locale-time`, off Unix, and on a platform libc has no `strftime_l` for, the
//! C locale is the only one: [`is_active`] is `false`.

#![allow(unsafe_code)] // FFI to the platform libc: `newlocale` and `strftime_l`

#[cfg(all(
    feature = "locale-time",
    any(target_vendor = "apple", target_os = "linux")
))]
mod imp {
    use alloc::string::String;
    use alloc::vec::Vec;
    use std::sync::OnceLock;

    use super::{TimeNames, TmFields};

    /// A `locale_t` that is never freed, so it can be shared by every thread.
    struct Handle(libc::locale_t);

    // SAFETY: the handle is created once, never mutated or freed, and `strftime_l` only
    // reads it.
    unsafe impl Send for Handle {}
    // SAFETY: as above.
    unsafe impl Sync for Handle {}

    static LOCALE: OnceLock<Option<Handle>> = OnceLock::new();

    /// The locale name jq's `setlocale(LC_ALL, "")` would settle on for `LC_TIME`:
    /// `LC_ALL`, else `LC_TIME`, else `LANG`, the first one that is set and non-empty.
    fn time_locale_name() -> Option<String> {
        ["LC_ALL", "LC_TIME", "LANG"].iter().find_map(|var| {
            // `var_os`: a value that is not UTF-8 still names the locale libc would pick.
            std::env::var_os(var)
                .filter(|v| !v.is_empty())
                .map(|v| v.to_string_lossy().into_owned())
        })
    }

    fn is_c_name(name: &str) -> bool {
        // `C.UTF-8` and `C.utf8` name the same English tables as `C`.
        name == "C" || name == "POSIX" || name.starts_with("C.")
    }

    fn resolve() -> Option<Handle> {
        // Cheap exit for the common case: no environment names a locale, or it names C.
        // (A conflicting category elsewhere, like `LC_CTYPE`, cannot make `LC_TIME`
        // localized when the name that wins for it is C.)
        let name = time_locale_name()?;
        if is_c_name(&name) {
            return None;
        }
        // SAFETY: an empty name and a null base are the documented arguments; the
        // result is either null (failure) or a handle we keep.
        let handle = unsafe {
            libc::newlocale(
                libc::LC_ALL_MASK,
                b"\0".as_ptr().cast(),
                core::ptr::null_mut(),
            )
        };
        if handle.is_null() {
            None
        } else {
            Some(Handle(handle))
        }
    }

    pub(super) fn is_active() -> bool {
        LOCALE.get_or_init(resolve).is_some()
    }

    static NAMES: OnceLock<Option<TimeNames>> = OnceLock::new();

    /// The locale's day, month and am/pm names, rendered by `strftime_l` -- the same source
    /// `strftime` prints from, so a name `strftime` writes is one `strptime` reads. `None`
    /// under C.
    pub(super) fn names() -> Option<&'static TimeNames> {
        NAMES.get_or_init(build_names).as_ref()
    }

    fn build_names() -> Option<TimeNames> {
        LOCALE.get_or_init(resolve).as_ref()?;
        // Only the field a specifier reads matters; the rest is a fixed ordinary date.
        let at = |month: i64, weekday: i64, hour: i64| TmFields {
            year: 2015,
            month,
            day: 1,
            hour,
            minute: 0,
            second: 0,
            weekday,
            yearday: 0,
            zone_offset: 0,
            zone_name: "UTC",
        };
        let weekdays = |spec: &str| -> Option<[String; 7]> {
            let mut out: [String; 7] = Default::default();
            for (i, slot) in out.iter_mut().enumerate() {
                *slot = render(spec, &at(1, i as i64, 0))?;
            }
            Some(out)
        };
        let months = |spec: &str| -> Option<[String; 12]> {
            let mut out: [String; 12] = Default::default();
            for (i, slot) in out.iter_mut().enumerate() {
                *slot = render(spec, &at(i as i64 + 1, 0, 0))?;
            }
            Some(out)
        };
        Some(TimeNames {
            weekday_full: weekdays("%A")?,
            weekday_abbr: weekdays("%a")?,
            month_full: months("%B")?,
            month_abbr: months("%b")?,
            am: render("%p", &at(1, 0, 0))?,
            pm: render("%p", &at(1, 0, 12))?,
        })
    }

    /// Render `spec` (for example `%A` or `%Ec`) with `strftime_l`. `None` when no
    /// non-C locale is active, or the fields do not fit a C `tm`.
    pub(super) fn render(spec: &str, tm: &TmFields<'_>) -> Option<String> {
        let handle = LOCALE.get_or_init(resolve).as_ref()?;
        let year = i32::try_from(tm.year.checked_sub(1900)?).ok()?;
        let zone = std::ffi::CString::new(tm.zone_name).ok()?;
        let spec = std::ffi::CString::new(spec).ok()?;
        // SAFETY: `libc::tm` is plain old data; all-zero is a valid value for every
        // field, the zone pointer included (null).
        let mut c_tm: libc::tm = unsafe { core::mem::zeroed() };
        c_tm.tm_sec = i32::try_from(tm.second).ok()?;
        c_tm.tm_min = i32::try_from(tm.minute).ok()?;
        c_tm.tm_hour = i32::try_from(tm.hour).ok()?;
        c_tm.tm_mday = i32::try_from(tm.day).ok()?;
        c_tm.tm_mon = i32::try_from(tm.month.checked_sub(1)?).ok()?;
        c_tm.tm_year = year;
        c_tm.tm_wday = i32::try_from(tm.weekday).ok()?;
        c_tm.tm_yday = i32::try_from(tm.yearday).ok()?;
        c_tm.tm_gmtoff = tm.zone_offset as _;
        // `tm_zone` is a `*mut` on Apple and a `*const` on glibc; the library only reads it.
        #[cfg(target_vendor = "apple")]
        {
            c_tm.tm_zone = zone.as_ptr().cast_mut();
        }
        #[cfg(not(target_vendor = "apple"))]
        {
            c_tm.tm_zone = zone.as_ptr();
        }

        // A localized name is at most a few dozen bytes and a layout a few more; 512
        // covers every specifier this renders with room to spare.
        let mut buf: Vec<u8> = alloc::vec![0; 512];
        // SAFETY: `buf` is writable for `buf.len()` bytes, `spec` is NUL-terminated,
        // `c_tm` is initialised and `zone` outlives the call.
        let written = unsafe {
            libc::strftime_l(
                buf.as_mut_ptr().cast(),
                buf.len(),
                spec.as_ptr(),
                &c_tm,
                handle.0,
            )
        };
        buf.truncate(written);
        // A locale that is not UTF-8 yields U+FFFD for what is not, as jq's output does.
        Some(String::from_utf8_lossy(&buf).into_owned())
    }
}

#[cfg(not(all(
    feature = "locale-time",
    any(target_vendor = "apple", target_os = "linux")
)))]
mod imp {
    use alloc::string::String;

    use super::{TimeNames, TmFields};

    pub(super) fn is_active() -> bool {
        false
    }

    pub(super) fn render(_spec: &str, _tm: &TmFields<'_>) -> Option<String> {
        None
    }

    pub(super) fn names() -> Option<&'static TimeNames> {
        None
    }
}

/// The broken-down time and zone one conversion is rendered for.
// The C-locale stub never reads the fields; only the `strftime_l` backend does.
#[cfg_attr(
    not(all(
        feature = "locale-time",
        any(target_vendor = "apple", target_os = "linux")
    )),
    allow(dead_code)
)]
pub(crate) struct TmFields<'a> {
    pub(crate) year: i64,
    /// 1-12, as `strftime` receives it.
    pub(crate) month: i64,
    pub(crate) day: i64,
    pub(crate) hour: i64,
    pub(crate) minute: i64,
    pub(crate) second: i64,
    pub(crate) weekday: i64,
    pub(crate) yearday: i64,
    pub(crate) zone_offset: i64,
    pub(crate) zone_name: &'a str,
}

/// The names `strptime` reads, in the process locale (#5754).
// The C-locale stub never builds one; only the `strftime_l` backend does.
#[cfg_attr(
    not(all(
        feature = "locale-time",
        any(target_vendor = "apple", target_os = "linux")
    )),
    allow(dead_code)
)]
pub(crate) struct TimeNames {
    /// Sunday first, as `tm_wday` counts.
    pub(crate) weekday_full: [alloc::string::String; 7],
    pub(crate) weekday_abbr: [alloc::string::String; 7],
    /// January first.
    pub(crate) month_full: [alloc::string::String; 12],
    pub(crate) month_abbr: [alloc::string::String; 12],
    /// What `%p` prints before and after noon. Either can be empty: macOS prints nothing for
    /// `%p` under `de_DE`, and its `strptime` then reads the empty string as the am marker.
    pub(crate) am: alloc::string::String,
    pub(crate) pm: alloc::string::String,
}

/// The locale's day, month and am/pm names, or `None` when `strptime` should read the
/// evaluator's own English ones (a C locale, no `locale-time`, and this crate's unit tests).
pub(crate) fn names() -> Option<&'static TimeNames> {
    if cfg!(test) {
        None
    } else {
        imp::names()
    }
}

/// Whether the locale's `%c`/`%x`/`%X`/`%r` layouts can print the zone (`%Z`, `%z`): glibc's
/// `en_GB` `%c` ends in `%Z`. A format using any of them has to resolve the zone labels.
pub(crate) fn layouts_may_name_the_zone(fmt: &str) -> bool {
    is_active() && fmt.bytes().any(|b| matches!(b, b'c' | b'x' | b'X' | b'r'))
}

/// Whether a non-C locale is in effect for the time conversions.
///
/// Always `false` in this crate's own unit tests, so their English expectations hold on a
/// host whose locale is not C; the localized path is exercised through the binary, with
/// the environment set, in `tests/jq_cli/i5754.rs`.
pub(crate) fn is_active() -> bool {
    !cfg!(test) && imp::is_active()
}

/// Render the conversion `spec` (`%A`, `%Ec`, ...) in the process locale, or `None` when
/// the evaluator's own C-locale rendering should run instead.
pub(crate) fn render(spec: &str, tm: &TmFields<'_>) -> Option<alloc::string::String> {
    imp::render(spec, tm)
}
