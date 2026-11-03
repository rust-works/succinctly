//! #5754: `strptime` reads a `%` specifier after a prefix only when the prefix does not end in a
//! modifier or a `+` flag. `%5%`, `%+0%` and `%E4%` read a `%`; `%+%`, `%E%`, `%E+%` and `%4E%` are
//! no match (a width makes the prefix end in digits, a bare modifier or `+` does not). Every
//! expectation was captured from jq 1.7.1 (`/usr/bin/jq`, macOS, `LC_ALL=C TZ=UTC`) by running the
//! same text through its `strptime`.

use std::process::Command;

use anyhow::Result;

use crate::{spawn_with_signal_retry, succinctly_bin};

/// `"%" | strptime($f) | .[0:6]` under `TZ=UTC` and `LC_ALL=C` as compact JSON, or `None` when jq
/// rejects the text (exit 5).
fn strptime_percent(fmt: &str) -> Result<Option<String>> {
    let (output, code) = spawn_with_signal_retry(
        || {
            let mut command = Command::new(succinctly_bin());
            command.env("TZ", "UTC").env("LC_ALL", "C").args([
                "jq",
                "-nc",
                "--arg",
                "f",
                fmt,
                "\"%\"|strptime($f)|.[0:6]",
            ]);
            command
        },
        None,
    )?;
    Ok(match code {
        0 => Some(String::from_utf8(output.stdout)?.trim_end().to_string()),
        5 => None,
        other => panic!("\"%\" | strptime({fmt:?}) exited {other}"),
    })
}

#[test]
fn a_percent_specifier_needs_a_width_after_a_modifier_or_plus() -> Result<()> {
    let read = Some("[1900,0,0,0,0,0]");
    for (fmt, want) in [
        ("%%", read),
        ("%0%", read),
        ("%5%", read),
        ("%+0%", read),
        ("%+4%", read),
        ("%E4%", read),
        ("%O5%", read),
        ("%E0%", read),
        ("%E+0%", read),
        ("%+O4%", read),
        ("%0E0%", read),
        ("%+%", None),
        ("%E%", None),
        ("%O%", None),
        ("%E+%", None),
        ("%+E%", None),
        ("%+O%", None),
        ("%4E%", None),
        ("%EE4%", None),
        ("%E++%", None),
        ("%4+%", None),
    ] {
        assert_eq!(
            strptime_percent(fmt)?.as_deref(),
            want,
            "\"%\" | strptime({fmt:?})"
        );
    }
    Ok(())
}
