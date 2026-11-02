//! #5754: `strftime` and `strflocaltime` take the `_`, `-` and `0` padding flags on the numeric
//! specifiers (`%_d` pads with a blank, `%-d` not at all, `%0e` with a zero), print a character
//! that names nothing bare (`%Q` is `Q`, `%5Y` is `5Y`), and fail with `unknown system failure`
//! when the result is empty or does not fit the `strlen(fmt) + 100` byte buffer jq gives the libc.
//! Every expectation is what jq 1.7.1 (`/usr/bin/jq`) prints on macOS: `^`, `#` and a width are
//! not flags there, and `y`, `Y`, `C`, `g` and `G` ignore the flags (a glibc `jq` pads and strips
//! those, reads `^` and `#`, and prints an unknown `%Q` as written).

use std::process::Command;

use anyhow::Result;

/// `fn($t) | fn($f)` as compact JSON in the zone `tz`: `Ok(Some(json))`, or `Ok(None)` and the
/// error text when jq fails (exit 5).
fn format_time(function: &str, secs: i64, tz: &str, fmt: &str) -> Result<(Option<String>, String)> {
    let out = Command::new(crate::succinctly_bin())
        .args([
            "jq",
            "-nc",
            "--arg",
            "f",
            fmt,
            &format!("{secs}|{function}($f)"),
        ])
        .env("LC_ALL", "C")
        .env("TZ", tz)
        .output()?;
    let stderr = String::from_utf8(out.stderr)?;
    Ok(match out.status.code() {
        Some(0) => (
            Some(String::from_utf8(out.stdout)?.trim_end().to_string()),
            stderr,
        ),
        Some(5) => (None, stderr),
        other => panic!("{function} {fmt:?} exited {other:?}: {stderr}"),
    })
}

#[test]
fn flags_pad_the_numeric_specifiers_and_nothing_else() -> Result<()> {
    for (function, secs, tz, fmt, want) in [
        ("strftime", 1420167845, "UTC", "[%_d]", "\"[ 2]\""),
        ("strftime", 1420167845, "UTC", "[%-d]", "\"[2]\""),
        ("strftime", 1420167845, "UTC", "[%0d]", "\"[02]\""),
        ("strftime", 1420167845, "UTC", "[%_H]", "\"[ 3]\""),
        ("strftime", 1420167845, "UTC", "[%-H]", "\"[3]\""),
        ("strftime", 1420167845, "UTC", "[%0H]", "\"[03]\""),
        ("strftime", 1420167845, "UTC", "[%_I]", "\"[ 3]\""),
        ("strftime", 1420167845, "UTC", "[%-I]", "\"[3]\""),
        ("strftime", 1420167845, "UTC", "[%0I]", "\"[03]\""),
        ("strftime", 1420167845, "UTC", "[%_j]", "\"[  2]\""),
        ("strftime", 1420167845, "UTC", "[%-j]", "\"[2]\""),
        ("strftime", 1420167845, "UTC", "[%0j]", "\"[002]\""),
        ("strftime", 1420167845, "UTC", "[%_k]", "\"[ 3]\""),
        ("strftime", 1420167845, "UTC", "[%-k]", "\"[3]\""),
        ("strftime", 1420167845, "UTC", "[%0k]", "\"[03]\""),
        ("strftime", 1420167845, "UTC", "[%_l]", "\"[ 3]\""),
        ("strftime", 1420167845, "UTC", "[%-l]", "\"[3]\""),
        ("strftime", 1420167845, "UTC", "[%0l]", "\"[03]\""),
        ("strftime", 1420167845, "UTC", "[%_m]", "\"[ 1]\""),
        ("strftime", 1420167845, "UTC", "[%-m]", "\"[1]\""),
        ("strftime", 1420167845, "UTC", "[%0m]", "\"[01]\""),
        ("strftime", 1420167845, "UTC", "[%_M]", "\"[ 4]\""),
        ("strftime", 1420167845, "UTC", "[%-M]", "\"[4]\""),
        ("strftime", 1420167845, "UTC", "[%0M]", "\"[04]\""),
        ("strftime", 1420167845, "UTC", "[%_S]", "\"[ 5]\""),
        ("strftime", 1420167845, "UTC", "[%-S]", "\"[5]\""),
        ("strftime", 1420167845, "UTC", "[%0S]", "\"[05]\""),
        ("strftime", 1420167845, "UTC", "[%_U]", "\"[ 0]\""),
        ("strftime", 1420167845, "UTC", "[%-U]", "\"[0]\""),
        ("strftime", 1420167845, "UTC", "[%0U]", "\"[00]\""),
        ("strftime", 1420167845, "UTC", "[%_V]", "\"[ 1]\""),
        ("strftime", 1420167845, "UTC", "[%-V]", "\"[1]\""),
        ("strftime", 1420167845, "UTC", "[%0V]", "\"[01]\""),
        ("strftime", 1420167845, "UTC", "[%_W]", "\"[ 0]\""),
        ("strftime", 1420167845, "UTC", "[%-W]", "\"[0]\""),
        ("strftime", 1420167845, "UTC", "[%0W]", "\"[00]\""),
        ("strftime", 1420167845, "UTC", "[%_e]", "\"[ 2]\""),
        ("strftime", 1420167845, "UTC", "[%-e]", "\"[2]\""),
        ("strftime", 1420167845, "UTC", "[%0e]", "\"[02]\""),
        ("strftime", 1420167845, "UTC", "%_a|%-b|%0c|%_p|%-y|%_Y|%0C|%_D", "\"Fri|Jan|Fri Jan  2 03:04:05 2015|AM|15|2015|20|01/02/15\""),
        ("strftime", 1420167845, "UTC", "%_%|%-%", "\"%|%\""),
        ("strftime", 1420167845, "UTC", "%__m|%_-m|%0-d", "\"_m|-m|-d\""),
        ("strftime", 1420167845, "UTC", "%^a|%#b|%5Y|%010d|%Q", "\"^a|#b|5Y|10d|Q\""),
        ("strftime", 1420167845, "UTC", "x%_", "\"x_\""),
        ("strftime", 1420167845, "UTC", "x%-", "\"x-\""),
        ("strftime", 1420167845, "UTC", "x%0", "\"x0\""),
        ("strftime", 1420167845, "UTC", "%_n%-t", "\"\\n\\t\""),
        ("strftime", 1420167845, "UTC", "%P|%Z|%_Z", "\"P|UTC|UTC\""),
        ("strftime", 1425599621, "UTC", "%_m/%-d %_H:%-M:%_S %_j %-j", "\" 3/5 23:53:41  64 64\""),
        ("strftime", 0, "UTC", "%_m/%-d %_H:%-M:%_S %_j %-j", "\" 1/1  0:0: 0   1 1\""),
        ("strftime", 1709287445, "UTC", "%_m/%-d %_H:%-M:%_S %_j %-j", "\" 3/1 10:4: 5  61 61\""),
        ("strflocaltime", 1420167845, "Asia/Kolkata", "%z", "\"+0530\""),
        ("strflocaltime", 1420167845, "Asia/Kolkata", "%_z", "\"+ 530\""),
        ("strflocaltime", 1420167845, "Asia/Kolkata", "%-z", "\"+530\""),
        ("strflocaltime", 1420167845, "Asia/Kolkata", "%0z", "\"+0530\""),
        ("strflocaltime", 1420167845, "America/St_Johns", "%z", "\"-0330\""),
        ("strflocaltime", 1420167845, "America/St_Johns", "%_z", "\"- 330\""),
        ("strflocaltime", 1420167845, "America/St_Johns", "%-z", "\"-330\""),
        ("strflocaltime", 1420167845, "America/St_Johns", "%0z", "\"-0330\""),
        ("strflocaltime", 1420167845, "America/New_York", "%z", "\"-0500\""),
        ("strflocaltime", 1420167845, "America/New_York", "%_z", "\"- 500\""),
        ("strflocaltime", 1420167845, "America/New_York", "%-z", "\"-500\""),
        ("strflocaltime", 1420167845, "America/New_York", "%0z", "\"-0500\""),
        ("strflocaltime", 1420167845, "UTC", "%z", "\"+0000\""),
        ("strflocaltime", 1420167845, "UTC", "%_z", "\"+   0\""),
        ("strflocaltime", 1420167845, "UTC", "%-z", "\"+0\""),
        ("strflocaltime", 1420167845, "UTC", "%0z", "\"+0000\""),
        ("strftime", 1420167845, "UTC", "%p", "\"AM\""),
        ("strftime", 1420167845, "UTC", "%c%c%c%c", "\"Fri Jan  2 03:04:05 2015Fri Jan  2 03:04:05 2015Fri Jan  2 03:04:05 2015Fri Jan  2 03:04:05 2015\""),
    ] {
        assert_eq!(
            format_time(function, secs, tz, fmt)?.0.as_deref(),
            Some(want),
            "{secs} | {function}({fmt:?}) in {tz}"
        );
    }
    Ok(())
}

#[test]
fn an_empty_or_oversized_result_is_a_system_failure() -> Result<()> {
    for (function, secs, tz, fmt) in [
        ("strftime", 1420167845, "UTC", ""),
        ("strftime", 1420167845, "UTC", "%c%c%c%c%c"),
        ("strflocaltime", 1420167845, "UTC", ""),
    ] {
        let (out, stderr) = format_time(function, secs, tz, fmt)?;
        assert_eq!(out, None, "{function}({fmt:?})");
        assert!(
            stderr.contains(&format!("{function}/1: unknown system failure")),
            "{function}({fmt:?}): {stderr}"
        );
    }
    Ok(())
}
