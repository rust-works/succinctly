//! #5863: yq's datetime operators. A duration is added to or subtracted from a timestamp
//! (`.t += "3h10m"` used to concatenate the text, exit 0), `format_datetime`, `tz` and `with_dtf`
//! exist, and `now`, `from_unix` and `to_unix` read and write timestamps as yq does, in the
//! local zone. Every row is `yq -I0 -o json FILTER` on [`DOC`] under the `TZ` it names, captured
//! from yq v4.53.3: stdout, stderr and the exit code. The rows in [`DIVERGENT`] are where
//! `docs/compliance/yq/limitations.md` records why succinctly answers differently.

use std::process::Command;

use anyhow::Result;

use crate::spawn_with_signal_retry;

const DOC: &str = "a: 1\nt: 2026-01-01T00:00:00Z\ne: \".a\"\nd: 2026-01-01\nq: \"2026-01-01T00:00:00Z\"\nn: 2026-01-01T10:20:30.5+05:30\np: 2026-01-01T00:00:00+00:00\nw: 2026-07-01T00:00:00+10:00\ns: 2026-04-04T23:30:00+11:00\n";

/// (TZ, filter, stdout, stderr, exit code), each the same as yq v4.53.3's.
const ROWS: &[(&str, &str, &str, &str, i32)] = &[
    // Two terms of exactly 1<<63 ns wrap Go's uint64 to 0 (a debug build once panicked here).
    ("UTC", ".t + \"9223372036854775808ns9223372036854775808ns\"", "\"2026-01-01T00:00:00Z\"\n", "", 0),
    // A signed zone abbreviation reaches 23 hours, and 24 is not one.
    ("UTC", "with_dtf(\"2006 MST\"; \"2026 +13\" | to_unix)", "1767225600\n", "", 0),
    ("UTC", "with_dtf(\"2006 MST\"; \"2026 +23\" | to_unix)", "1767225600\n", "", 0),
    ("UTC", "with_dtf(\"2006 MST\"; \"2026 +24\" | to_unix)", "", "Error: could not parse datetime of [] using layout [2006 MST]: parsing time \"2026 +24\" as \"2006 MST\": cannot parse \"+24\" as \"MST\"", 1),
    // Go's "no offset" marker is -1, so an offset of -1s is read as none (UTC) and -2s is a zone.
    ("UTC", "with_dtf(\"15:04:05 -07:00:00\"; \"10:00:00 -00:00:01\" | to_unix)", "-62167183200\n", "", 0),
    ("UTC", "with_dtf(\"15:04:05 -07:00:00\"; \"10:00:00 -00:00:02\" | to_unix)", "-62167183198\n", "", 0),
    // A negative offset under a minute keeps its sign in the seconds field.
    ("UTC", "with_dtf(\"2006-01-02 15:04:05 -07:00:00\"; \"2026-01-01 10:00:00 -00:00:30\" | format_datetime(\"-07:00:00\"))", "\"+00:00:-30\"\n", "", 0),
    ("UTC", "with_dtf(\"2006-01-02 15:04:05 -07:00:00\"; \"2026-01-01 10:00:00 -00:00:30\" | format_datetime(\"-0700\"))", "0\n", "", 0),
    // Every error from the layout argument is prefixed, not only a missing layout.
    ("UTC", "with_dtf(error(\"boom\"); .t)", "", "Error: could not get date time format: boom", 1),
    ("UTC", ".t += \"3h10m\" | .t", "\"2026-01-01T03:10:00Z\"\n", "", 0),
    ("UTC", ".t -= \"24h\" | .t", "\"2025-12-31T00:00:00Z\"\n", "", 0),
    ("UTC", ".t | format_datetime(\"2006\")", "2026\n", "", 0),
    ("UTC", ".t | tz(\"UTC\")", "\"2026-01-01T00:00:00Z\"\n", "", 0),
    ("UTC", "with_dtf(\"2006\"; .t)", "\"2026-01-01T00:00:00Z\"\n", "", 0),
    ("UTC", ".t + \"3h\"", "\"2026-01-01T03:00:00Z\"\n", "", 0),
    ("UTC", ".d + \"25h\"", "\"2026-01-02T01:00:00Z\"\n", "", 0),
    ("UTC", ".n + \"1h\"", "\"2026-01-01T11:20:30+05:30\"\n", "", 0),
    ("UTC", ".n - \"1h\"", "\"2026-01-01T09:20:30+05:30\"\n", "", 0),
    ("UTC", ".p + \"0s\"", "\"2026-01-01T00:00:00Z\"\n", "", 0),
    ("UTC", ".t += \"-1h\" | .t", "\"2025-12-31T23:00:00Z\"\n", "", 0),
    ("UTC", ".t -= \"-1h\" | .t", "\"2026-01-01T01:00:00Z\"\n", "", 0),
    ("UTC", ".t + \"1.5h\"", "\"2026-01-01T01:30:00Z\"\n", "", 0),
    ("UTC", ".t + \"1h\" + \"1h\"", "\"2026-01-01T02:00:00Z\"\n", "", 0),
    ("UTC", ".t |= . + \"1h\" | .t", "\"2026-01-01T01:00:00Z\"\n", "", 0),
    ("UTC", ".t - \"bogus\"", "", "Error: unable to parse duration [bogus]: time: invalid duration \"-bogus\"", 1),
    ("UTC", ".t + 1", "", "Error: unable to parse duration [1]: time: missing unit in duration \"1\"", 1),
    ("UTC", ".t - 1", "", "Error: unable to parse duration [1]: time: missing unit in duration \"-1\"", 1),
    ("UTC", "\"2026-01-01\" + \"T00:00:00Z\"", "\"2026-01-01T00:00:00Z\"\n", "", 0),
    ("UTC", ".t + \"3hrs\"", "", "Error: unable to parse duration [3hrs]: time: unknown unit \"hrs\" in duration \"3hrs\"", 1),
    ("UTC", "with_dtf(\"\"; .t + \"1h\")", "\"2026-01-01T01:00:00Z\"\n", "", 0),
    ("UTC", ".t + \"9999999h\"", "", "Error: unable to parse duration [9999999h]: time: invalid duration \"9999999h\"", 1),
    ("UTC", ".t - .t", "", "Error: unable to parse duration [2026-01-01T00:00:00Z]: time: unknown unit \"-\" in duration \"-2026-01-01T00:00:00Z\"", 1),
    ("UTC", ".e + \"x\"", "\".ax\"\n", "", 0),
    ("UTC", ".e - \"1h\"", "", "Error: strings cannot be subtracted", 1),
    ("UTC", ".t | format_datetime(\"Monday, 02-Jan-06 at 3:04PM MST\")", "\"Thursday, 01-Jan-26 at 12:00AM UTC\"\n", "", 0),
    ("UTC", ".n | format_datetime(\"Mon Jan _2 15:04:05.000 -07:00 2006\")", "\"Thu Jan  1 10:20:30.500 +05:30 2026\"\n", "", 0),
    ("UTC", ".t | format_datetime(\"[2006, 01]\")", "[2026,1]\n", "", 0),
    ("UTC", ".t | format_datetime(\"a: 2006\")", "{\"a\":2026}\n", "", 0),
    ("UTC", ".t | format_datetime(\"x\")", "\"x\"\n", "", 0),
    ("UTC", ".t | format_datetime(\"\")", "null\n", "", 0),
    ("UTC", ".t | format_datetime(\"\\\"2006\\\"\")", "\"\\\"2026\\\"\"\n", "", 0),
    ("UTC", ".q | format_datetime(\"2006\")", "2026\n", "", 0),
    ("UTC", ".t | format_datetime(1)", "1\n", "", 0),
    ("UTC", ".t | format_datetime(\"2006\", \"01\")", "2026\n", "", 0),
    ("UTC", ".t | tz(\"Australia/Sydney\")", "\"2026-01-01T11:00:00+11:00\"\n", "", 0),
    ("UTC", ".n | tz(\"America/New_York\")", "\"2025-12-31T23:50:30-05:00\"\n", "", 0),
    ("UTC", ".t | tz(\"\")", "\"2026-01-01T00:00:00Z\"\n", "", 0),
    ("UTC", ".t | tz(\"Nope/Zone\")", "", "Error: could not load tz [Nope/Zone]: unknown time zone Nope/Zone", 1),
    ("UTC", ".t | tz(\"../x\")", "", "Error: could not load tz [../x]: time: invalid location name", 1),
    ("UTC", ".q | tz(\"UTC\")", "\"2026-01-01T00:00:00Z\"\n", "", 0),
    ("UTC", ".t | tz(\"Australia/Sydney\") | format_datetime(\"MST -0700\")", "\"+1100 +1100\"\n", "", 0),
    ("UTC", ".t | tz(\"Australia/Sydney\") | . + \"1h\"", "\"2026-01-01T12:00:00+11:00\"\n", "", 0),
    ("UTC", "with_dtf(\"2006\"; \"2001\" + \"1h\")", "\"2001\"\n", "", 0),
    ("UTC", "with_dtf(\"2006\"; \"2001\" - \"1h\")", "\"2000\"\n", "", 0),
    ("UTC", "with_dtf(\"2006\"; \"x\" + \"1h\")", "\"x1h\"\n", "", 0),
    ("UTC", "with_dtf(\"2006\"; \"x\" - \"1h\")", "", "Error: strings cannot be subtracted", 1),
    ("UTC", "with_dtf(\"2006\"; .t + \"1h\")", "", "Error: parsing time \"2026-01-01T00:00:00Z\": extra text: \"-01-01T00:00:00Z\"", 1),
    ("UTC", "with_dtf(\"2006-01-02\"; .d + \"25h\")", "\"2026-01-02\"\n", "", 0),
    ("UTC", "with_dtf(\"2006-01-02T15:04:05Z07:00\"; .d + \"1h\")", "\"2026-01-01T01:00:00Z\"\n", "", 0),
    ("UTC", "with_dtf(\"Jan 2006\"; \"Jan 2001\" < \"Feb 2002\")", "true\n", "", 0),
    ("UTC", "with_dtf(\"2006\"; .t < \"2002\")", "", "Error: parsing time \"2026-01-01T00:00:00Z\": extra text: \"-01-01T00:00:00Z\"", 1),
    ("UTC", "with_dtf(\"2006\"; \"2001\" | to_unix)", "978307200\n", "", 0),
    ("UTC", "with_dtf(\"2006\"; \"2001\" | format_datetime(\"Jan 2006\"))", "\"Jan 2001\"\n", "", 0),
    ("UTC", "with_dtf(\"2006\"; \"2001\" | tz(\"Australia/Sydney\"))", "\"2001\"\n", "", 0),
    ("UTC", "with_dtf(\"2006\"; with_dtf(\"2006-01-02\"; .d + \"1h\"))", "\"2026-01-01\"\n", "", 0),
    ("UTC", "with_dtf(\"2006\"; now) | to_unix > 1700000000", "true\n", "", 0),
    ("UTC", "with_dtf(.a; 1)", "1\n", "", 0),
    ("UTC", "with_dtf(\"2006\"; .a) = 5 | .a", "5\n", "", 0),
    ("UTC", "with_dtf(\"2006\"; .t) |= . + \"1h\" | .t", "\"2026-01-01T01:00:00Z\"\n", "", 0),
    ("UTC", "with_dtf(\"2006\"; .x.y) = 5 | .x", "{\"y\":5}\n", "", 0),
    ("UTC", "del(with_dtf(\"2006\"; .a)) | has(\"a\")", "false\n", "", 0),
    ("UTC", "0 | from_unix", "\"1970-01-01T00:00:00Z\"\n", "", 0),
    ("UTC", ".a - 2.5 | from_unix", "\"1969-12-31T23:59:58Z\"\n", "", 0),
    ("UTC", "1.9 | from_unix", "\"1970-01-01T00:00:01Z\"\n", "", 0),
    ("UTC", ".t | to_unix", "1767225600\n", "", 0),
    ("UTC", ".n | to_unix", "1767243030\n", "", 0),
    ("UTC", "\"2026-01-01T00:00:00\" | to_unix", "", "Error: could not parse datetime of [] using layout [2006-01-02T15:04:05Z07:00]: parsing time \"2026-01-01T00:00:00\": extra text: \"T00:00:00\"", 1),
    ("UTC", "now | format_datetime(\"2006\") >= 2026", "true\n", "", 0),
    ("UTC", "now | format_datetime(\"Z07:00\")", "\"Z\"\n", "", 0),
    ("UTC", "now | . + \"1h\" | to_unix > (now | to_unix)", "true\n", "", 0),
    ("UTC", "(.t | to_unix) as $x | .t | . - \"1s\" | to_unix == $x - 1", "true\n", "", 0),
    ("UTC", ".t < .n", "true\n", "", 0),
    ("Australia/Sydney", "0 | from_unix", "\"1970-01-01T10:00:00+10:00\"\n", "", 0),
    ("Australia/Sydney", ".t | tz(\"Local\")", "\"2026-01-01T11:00:00+11:00\"\n", "", 0),
    ("Australia/Sydney", ".t | format_datetime(\"MST\")", "\"UTC\"\n", "", 0),
    ("Australia/Sydney", ".p | format_datetime(\"MST\")", "0\n", "", 0),
    ("Australia/Sydney", "\"2026-01-01T11:00:00+11:00\" | format_datetime(\"MST\")", "\"AEDT\"\n", "", 0),
    ("Australia/Sydney", "\"2026-07-01T10:00:00+10:00\" | format_datetime(\"MST -0700\")", "\"AEST +1000\"\n", "", 0),
    ("Australia/Sydney", ".s + \"24h\"", "\"2026-04-05T22:30:00+10:00\"\n", "", 0),
    ("Australia/Sydney", ".s | format_datetime(\"MST\")", "\"AEDT\"\n", "", 0),
    ("Australia/Sydney", "with_dtf(\"2006-01-02 15:04 MST\"; \"2026-07-01 11:00 AEDT\" | to_unix)", "1782864000\n", "", 0),
    ("Australia/Sydney", "with_dtf(\"2006-01-02 15:04 MST\"; \"2026-07-01 11:00 AEST\" | to_unix)", "1782867600\n", "", 0),
    ("Australia/Sydney", "with_dtf(\"2006-01-02 15:04 MST\"; \"2026-01-01 11:00 PST\" | format_datetime(\"15:04 MST -0700\"))", "\"11:00 PST +0000\"\n", "", 0),
    ("Australia/Sydney", "with_dtf(\"2006-01-02 15:04 MST\"; \"2026-01-01 11:00 GMT+3\" | format_datetime(\"15:04 MST -0700\"))", "\"14:00 GMT+3 +0300\"\n", "", 0),
];

/// (TZ, filter, stdout, stderr, exit code) where succinctly differs from yq, with yq's answer in
/// the comment: the node's path in an error message (a value here carries none), a quoted string
/// spelled like a timestamp, which is one here, and a plain one, which concatenates text that
/// cannot begin a duration as a quoted one does (`OwnedValue` has no timestamp tag).
const DIVERGENT: &[(&str, &str, &str, &str, i32)] = &[
    // yq: Error: unable to parse duration [bogus]: time: invalid duration "bogus" (text that cannot begin a duration concatenates, as for a quoted date)
    ("UTC", ".t + \"bogus\"", "\"2026-01-01T00:00:00Zbogus\"\n", "", 0),
    // yq: Error: unable to parse duration [null]: time: invalid duration "null" (text that cannot begin a duration concatenates, as for a quoted date)
    ("UTC", ".t + null", "\"2026-01-01T00:00:00Z\"\n", "", 0),
    // yq: Error: unable to parse duration [true]: time: invalid duration "true" (text that cannot begin a duration concatenates, as for a quoted date)
    ("UTC", ".t + true", "\"2026-01-01T00:00:00Ztrue\"\n", "", 0),
    // yq: Error: unable to parse duration []: time: invalid duration "" (text that cannot begin a duration concatenates, as for a quoted date)
    ("UTC", ".t + \"\"", "\"2026-01-01T00:00:00Z\"\n", "", 0),
    // yq: "!!timestamp" (the tag; `now` is text here, and a string's tag is `!!str`)
    ("UTC", "now | type", "\"!!str\"\n", "", 0),
    // yq: "2026-01-01T00:00:00Z3h" (a quoted string is `!!str` in yq, and concatenates)
    ("UTC", ".q + \"3h\"", "\"2026-01-01T03:00:00Z\"\n", "", 0),
    // yq: ["a"] (a stage after `with_dtf` reads its input's position)
    ("UTC", "with_dtf(\"2006\"; .a) | path", "[]\n", "", 0),
    // yq: Error: could not parse datetime of [e]: parsing time ".a" as "2006-01-02": cannot parse ".a" as "2006"
    ("UTC", ".e | format_datetime(\"2006\")", "", "Error: could not parse datetime of []: parsing time \".a\" as \"2006-01-02\": cannot parse \".a\" as \"2006\"", 1),
    // yq: "2026-01-01T00:00:00Z1h"
    ("UTC", "with_dtf(\"2006-01-02\"; .q + \"1h\")", "", "Error: parsing time \"2026-01-01T00:00:00Z\": extra text: \"T00:00:00Z\"", 1),
    // yq: Error: could not parse datetime of [e] using layout [2006-01-02T15:04:05Z07:00]: parsing time ".a" as "2006-01-02": cannot parse ".a" as "2006"
    ("UTC", ".e | to_unix", "", "Error: could not parse datetime of [] using layout [2006-01-02T15:04:05Z07:00]: parsing time \".a\" as \"2006-01-02\": cannot parse \".a\" as \"2006\"", 1),
];

fn run_in_zone(zone: &str, filter: &str) -> Result<(String, String, i32)> {
    let (output, code) = spawn_with_signal_retry(
        || {
            let mut command = Command::new(env!("CARGO_BIN_EXE_succinctly"));
            command
                .env("TZ", zone)
                .args(["yq", "-I0", "-o", "json", filter, "-"]);
            command
        },
        Some(DOC.as_bytes()),
    )?;
    Ok((
        String::from_utf8(output.stdout)?,
        String::from_utf8(output.stderr)?.trim_end().to_string(),
        code,
    ))
}

#[test]
fn test_datetime_operators_match_yq_5863() -> Result<()> {
    let mut failures = Vec::new();
    for &(zone, filter, stdout, stderr, code) in ROWS.iter().chain(DIVERGENT) {
        let got = run_in_zone(zone, filter)?;
        if (got.0.as_str(), got.1.as_str(), got.2) != (stdout, stderr, code) {
            failures.push(format!("[TZ={zone}] {filter}: got {got:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

/// The headline row: a timestamp written back to the document stays a plain timestamp, so the
/// YAML reads back as the same type.
#[test]
fn test_timestamp_plus_equals_writes_the_document_back_5863() -> Result<()> {
    let (output, code) = spawn_with_signal_retry(
        || {
            let mut command = Command::new(env!("CARGO_BIN_EXE_succinctly"));
            command
                .env("TZ", "UTC")
                .args(["yq", ".t += \"3h10m\" | .q += \"x\"", "-"]);
            command
        },
        Some(b"t: 2026-01-01T00:00:00Z\nq: \"a\"\n"),
    )?;
    assert_eq!(
        (String::from_utf8(output.stdout)?.as_str(), code),
        ("t: 2026-01-01T03:10:00Z\nq: \"ax\"\n", 0)
    );
    Ok(())
}
