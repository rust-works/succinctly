//! #7809: `strflocaltime` of a reading the zone skipped.
//!
//! Where `isdst` is 0 on both sides of the skip (a zone moving its standard offset: Europe/Moscow
//! on 2011-03-27, Pacific/Chatham in 1946, America/Santiago in 1916) the macOS C library's `mktime`
//! with `tm_isdst = -1` fails, so jq 1.7.1 and jq 1.8.2 print the fields as given and `%s` as `-1`.
//! `succinctly jq` carries the reading past the skip, as it does for a spring-forward gap (#7606),
//! and prints a real time and its epoch seconds. The difference is recorded in
//! `docs/compliance/jq/limitations.md` as a platform divergence (ADR-0018: the macOS answer is a
//! time the zone never had, with an epoch of -1); a glibc jq was not captured. These rows pin
//! `succinctly jq`'s answers, not the reference's: the macOS jq 1.7.1 (`/usr/bin/jq`) and jq 1.8.2
//! (`/opt/homebrew/bin/jq`) answers are in the comments.
//!
//! The last group is a narrower difference the same scan found: a gap in which `mktime` succeeds
//! but lands on a different instant from this implementation's (a half-hour or a whole-day skip).

use std::process::Command;

use anyhow::Result;

use crate::{spawn_with_signal_retry, succinctly_bin};

const FORMAT: &str = "%Y-%m-%d %H:%M:%S %Z %s";

/// (TZ, broken-down time, stdout of `strflocaltime($f)`).
const ROWS: &[(&str, &str, &str)] = &[
    // The three cases of the issue. The macOS jqs print `"2011-03-27 02:30:00 MSK -1"` (both),
    // `"1946-01-01 00:15:00 +1245 -1"` (1.7.1) and `"... +1215 -1"` (1.8.2), `"1916-07-01
    // 00:10:00 -04 -1"` (1.7.1) and `"... SMT -1"` (1.8.2).
    (
        "Europe/Moscow",
        "[2011,2,27,2,30,0,0,85]",
        "\"2011-03-27 03:30:00 MSK 1301182200\"\n",
    ),
    (
        "Pacific/Chatham",
        "[1946,0,1,0,15,0,0,0]",
        "\"1946-01-01 00:45:00 +1245 -757425600\"\n",
    ),
    (
        "America/Santiago",
        "[1916,6,1,0,10,0,0,182]",
        "\"1916-07-01 00:27:15 SMT -1688410200\"\n",
    ),
    // Others found by the scan (isdst 0 on both sides): macOS prints the fields as given and -1.
    (
        "Asia/Kabul",
        "[1945,0,1,0,15,0,0,0]",
        "\"1945-01-01 00:45:00 +0430 -788931900\"\n",
    ),
    (
        "Europe/Andorra",
        "[1946,8,30,0,30,0,1,272]",
        "\"1946-09-30 01:30:00 CET -733879800\"\n",
    ),
    // The edges of the Moscow skip and the reading before it: the same in all three.
    (
        "Europe/Moscow",
        "[2011,2,27,1,59,59,0,85]",
        "\"2011-03-27 01:59:59 MSK 1301180399\"\n",
    ),
    (
        "Europe/Moscow",
        "[2011,2,27,3,0,0,0,85]",
        "\"2011-03-27 03:00:00 MSK 1301180400\"\n",
    ),
    // A gap where `isdst` changes works as the #7606 section says: jq 1.8.2 and this agree
    // (jq 1.7.1 formats the fields as given, `02:30:00 EST`).
    (
        "America/New_York",
        "[2024,2,10,2,30,0,0,69]",
        "\"2024-03-10 03:30:00 EDT 1710055800\"\n",
    ),
    // Narrower: `mktime` succeeds on macOS but picks another instant. jq 1.8.2 prints
    // `"2011-12-29 01:00:00 -10 1325156400"` (the skipped day is 2011-12-30, which Samoa did not
    // have), `"1928-10-14 03:00:00 NZST -1300698000"` (a half-hour gap) and
    // `"1941-12-24 23:30:00 HKWT -884250000"` (a half-hour gap where `isdst` goes from 1 to 0);
    // jq 1.7.1 has the same epoch seconds in each.
    (
        "Pacific/Apia",
        "[2011,11,30,0,0,0,0,363]",
        "\"2011-12-31 00:00:00 +14 1325239200\"\n",
    ),
    (
        "Antarctica/McMurdo",
        "[1928,9,14,2,0,0,0,287]",
        "\"1928-10-14 02:30:00 NZST -1300699800\"\n",
    ),
    (
        "Asia/Hong_Kong",
        "[1941,11,25,0,0,0,0,358]",
        "\"1941-12-25 00:30:00 JST -884248200\"\n",
    ),
];

#[test]
fn a_reading_in_a_skip_that_leaves_isdst_unchanged_is_carried_past_it_7809() -> Result<()> {
    for &(tz, value, want) in ROWS {
        let program = format!("{value} | strflocaltime($f)");
        let (output, code) = spawn_with_signal_retry(
            || {
                let mut command = Command::new(succinctly_bin());
                command
                    .env("TZ", tz)
                    .args(["jq", "-nc", "--arg", "f", FORMAT, &program]);
                command
            },
            None,
        )?;
        let stdout = String::from_utf8(output.stdout)?;
        assert_eq!((stdout.as_str(), code), (want, 0), "TZ={tz} {program}");
    }
    Ok(())
}
