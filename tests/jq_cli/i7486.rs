//! #7486: `localtime` and `strflocaltime` read `TZ` through jiff's zone database, so
//! `TZ=posix/Asia/Tokyo`, `TZ=right/...` and `TZ=Asia//Tokyo` were UTC, `TZDIR` was not read as
//! glibc reads it, and names matched whatever their case. libc reads `TZ` as a tz file under the
//! zoneinfo directory (or at a path) before it reads it as a POSIX string. Every row is jq 1.7.1's
//! output, captured on x86_64 Linux (glibc, `~/.local/bin/jq`) and on macOS arm64
//! (`/usr/bin/jq`; jq 1.8.2 printed the same there).
//!
//! The rows need the machine's zoneinfo (`Asia/Tokyo`, and on Linux the `posix/` tree): a machine
//! without it skips them. Linux and macOS only, the two libcs captured.

#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::path::Path;
use std::process::Command;

/// The zones' labels and their local times as `mktime` reads them back.
const LABELLED: &str = r#"[1700000000, 1483228826] | [(.[]|strflocaltime("%Y-%m-%dT%H:%M:%S %Z")), map(localtime|mktime)]"#;

/// The local times only, for the rows where glibc labels a zone it could not read with the name's
/// letters (`asia`), which `succinctly jq` labels `UTC` (a recorded label difference).
const VALUES: &str = "[1700000000, 1483228826] | map(localtime|mktime)";

const TOKYO: &str =
    r#"["2023-11-15T07:13:20 JST","2017-01-01T09:00:26 JST",[1700032400,1483261226]]"#;
const NEW_YORK: &str =
    r#"["2023-11-14T17:13:20 EST","2016-12-31T19:00:26 EST",[1699982000,1483210826]]"#;
const PARIS: &str =
    r#"["2023-11-14T23:13:20 CET","2017-01-01T01:00:26 CET",[1700003600,1483232426]]"#;
const UTC_LABELLED: &str =
    r#"["2023-11-14T22:13:20 UTC","2017-01-01T00:00:26 UTC",[1700000000,1483228826]]"#;
const UTC_VALUES: &str = "[1700000000,1483228826]";
const TOKYO_VALUES: &str = "[1700032400,1483261226]";
const NEW_YORK_VALUES: &str = "[1699982000,1483210826]";

/// `TZDIR` for a row: unset, the test's own directory, or as given.
#[derive(Clone, Copy)]
enum Dir {
    Unset,
    Own,
    Is(&'static str),
}

/// `(TZDIR, TZ, program, jq 1.7.1 on glibc, jq 1.7.1 on macOS)`.
const ROWS: &[(Dir, &str, &str, &str, &str)] = &[
    (
        Dir::Unset,
        "posix/Asia/Tokyo",
        LABELLED,
        TOKYO,
        UTC_LABELLED,
    ),
    (
        Dir::Unset,
        ":posix/Asia/Tokyo",
        LABELLED,
        TOKYO,
        UTC_LABELLED,
    ),
    (
        Dir::Unset,
        "posix/Europe/Paris",
        LABELLED,
        PARIS,
        UTC_LABELLED,
    ),
    (Dir::Unset, "Asia//Tokyo", LABELLED, TOKYO, TOKYO),
    (Dir::Unset, "Asia/./Tokyo", LABELLED, TOKYO, TOKYO),
    (Dir::Unset, "Asia/../Asia/Tokyo", LABELLED, TOKYO, TOKYO),
    (Dir::Unset, "./Asia/Tokyo", LABELLED, TOKYO, TOKYO),
    (
        Dir::Unset,
        "/usr/share/zoneinfo/Asia/Tokyo",
        LABELLED,
        TOKYO,
        TOKYO,
    ),
    (
        Dir::Unset,
        "/usr/share/zoneinfo//Asia/Tokyo",
        LABELLED,
        TOKYO,
        TOKYO,
    ),
    (
        Dir::Unset,
        "//usr/share/zoneinfo/Asia/Tokyo",
        LABELLED,
        TOKYO,
        TOKYO,
    ),
    (
        Dir::Unset,
        ":/usr/share/zoneinfo/Asia/Tokyo",
        LABELLED,
        TOKYO,
        TOKYO,
    ),
    (Dir::Unset, "posixrules", LABELLED, NEW_YORK, NEW_YORK),
    (Dir::Unset, "Japan", LABELLED, TOKYO, TOKYO),
    (
        Dir::Unset,
        "Factory",
        LABELLED,
        r#"["2023-11-14T22:13:20 -00","2017-01-01T00:00:26 -00",[1700000000,1483228826]]"#,
        r#"["2023-11-14T22:13:20 -00","2017-01-01T00:00:26 -00",[1700000000,1483228826]]"#,
    ),
    (Dir::Unset, "JST-9", LABELLED, TOKYO, TOKYO),
    // glibc drops the `:` and reads the POSIX string; Apple's libc never reads a `:` name as one.
    (Dir::Unset, ":JST-9", LABELLED, TOKYO, UTC_LABELLED),
    (
        Dir::Unset,
        "XYZ5XYD",
        LABELLED,
        r#"["2023-11-14T17:13:20 XYZ","2016-12-31T19:00:26 XYZ",[1699982000,1483210826]]"#,
        r#"["2023-11-14T17:13:20 XYZ","2016-12-31T19:00:26 XYZ",[1699982000,1483210826]]"#,
    ),
    // A name matches a file as the file system matches it: case-sensitive on Linux, not on macOS.
    (Dir::Unset, "asia/tokyo", VALUES, UTC_VALUES, TOKYO_VALUES),
    (Dir::Unset, "ASIA/TOKYO", VALUES, UTC_VALUES, TOKYO_VALUES),
    (
        Dir::Unset,
        "america/new_york",
        VALUES,
        UTC_VALUES,
        NEW_YORK_VALUES,
    ),
    (Dir::Unset, "etc/gmt+5", VALUES, UTC_VALUES, NEW_YORK_VALUES),
    (
        Dir::Unset,
        "posix/asia/tokyo",
        VALUES,
        UTC_VALUES,
        UTC_VALUES,
    ),
    // glibc reads `TZDIR`; Apple's libc does not. The test's directory holds `My/Zone` (Tokyo),
    // `Asia/Tokyo` (New York) and `Paris` (Europe/Paris).
    (Dir::Own, "My/Zone", LABELLED, TOKYO, UTC_LABELLED),
    (Dir::Own, "Asia/Tokyo", LABELLED, NEW_YORK, TOKYO),
    (Dir::Own, "Paris", LABELLED, PARIS, UTC_LABELLED),
    (
        Dir::Own,
        "/usr/share/zoneinfo/Asia/Tokyo",
        LABELLED,
        TOKYO,
        TOKYO,
    ),
    (Dir::Own, "EST5EDT", LABELLED, NEW_YORK, NEW_YORK),
    (
        Dir::Is("/nonexistent"),
        "Asia/Tokyo",
        VALUES,
        UTC_VALUES,
        TOKYO_VALUES,
    ),
    (
        Dir::Is("/nonexistent"),
        "posix/Asia/Tokyo",
        VALUES,
        UTC_VALUES,
        UTC_VALUES,
    ),
    (
        Dir::Is("/nonexistent"),
        "Japan",
        VALUES,
        UTC_VALUES,
        TOKYO_VALUES,
    ),
    (
        Dir::Is("/usr/share/zoneinfo/posix"),
        "Asia/Tokyo",
        LABELLED,
        TOKYO,
        TOKYO,
    ),
    // An empty `TZDIR` is unset.
    (Dir::Is(""), "Asia/Tokyo", LABELLED, TOKYO, TOKYO),
];

#[test]
fn tz_is_read_as_libc_reads_it_7486() {
    let zoneinfo = Path::new("/usr/share/zoneinfo");
    let linux = cfg!(target_os = "linux");
    let needed = ["Asia/Tokyo", "America/New_York", "Europe/Paris"];
    if needed.iter().any(|zone| !zoneinfo.join(zone).is_file())
        || (linux && !zoneinfo.join("posix/Asia/Tokyo").is_file())
    {
        return;
    }
    let own = tempfile::tempdir().expect("tempdir");
    for (zone, as_name) in [
        ("Asia/Tokyo", "My/Zone"),
        ("America/New_York", "Asia/Tokyo"),
        ("Europe/Paris", "Paris"),
    ] {
        let to = own.path().join(as_name);
        std::fs::create_dir_all(to.parent().unwrap()).expect("created");
        std::fs::copy(zoneinfo.join(zone), to).expect("copied");
    }
    let mut failures = Vec::new();
    for &(dir, tz, program, glibc, macos) in ROWS {
        let mut command = Command::new(env!("CARGO_BIN_EXE_succinctly"));
        command
            .args(["jq", "-n", "-c", program])
            .env_remove("TZDIR")
            .env("TZ", tz);
        match dir {
            Dir::Unset => {}
            Dir::Own => {
                command.env("TZDIR", own.path());
            }
            Dir::Is(path) => {
                command.env("TZDIR", path);
            }
        }
        let output = command.output().expect("ran");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let expected = if linux { glibc } else { macos };
        if stdout.trim_end() != expected || !output.status.success() {
            failures.push(format!(
                "TZ={tz:?} TZDIR={}: got {stdout:?}, want {expected:?}",
                match dir {
                    Dir::Unset => "<unset>".to_string(),
                    Dir::Own => "<own>".to_string(),
                    Dir::Is(path) => format!("{path:?}"),
                }
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
