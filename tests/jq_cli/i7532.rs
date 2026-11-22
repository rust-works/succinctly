//! #7532: `mktime`, `strftime`, `strflocaltime`, `todate` and `todateiso8601` read a broken-down
//! time as jq 1.8.2's `jv2tm` does (upstream fixes jqlang/jq#3070 and #3162): a NaN in any of the
//! first 8 fields is `<name> requires parsed datetime inputs` (so is a NaN number input, whose
//! `gmtime` seconds slot is NaN), an array shorter than 8 is read with its missing fields 0, and each
//! field is truncated and saturated to C `int` bounds, the year after 1900 is taken from it, so the
//! year saturates at `2147485547` and never wraps. jq 1.7.1 read NaN as 0, required all 8 fields and
//! cast each one to `int` unchecked (undefined behaviour in C).
//!
//! Every row was captured from jq 1.8.2 (`/opt/homebrew/bin/jq`) on macOS under `TZ=UTC`. The rows
//! avoid what the C library decides there: no `%Z`/`%z`/`%s`, no date before 1900 (macOS's
//! `timegm` refuses one, glibc's converts it), and no field that jq 1.8.2's normalising `timegm`
//! call (jqlang/jq#3203, which succinctly does not follow) would move: every date given is a real
//! one, and `%a`/`%j` are not printed.

use std::process::Command;

use anyhow::Result;

use crate::{spawn_with_signal_retry, succinctly_bin};

/// (value, filter, stdout of `try (value | filter) catch .`).
const ROWS: &[(&str, &str, &str)] = &[
    (
        "[nan,0,1,0,0,0,0,0]",
        "mktime",
        "\"mktime requires parsed datetime inputs\"\n",
    ),
    (
        "[2024,nan,1,0,0,0,0,0]",
        "mktime",
        "\"mktime requires parsed datetime inputs\"\n",
    ),
    (
        "[2024,0,1,0,0,nan,0,0]",
        "mktime",
        "\"mktime requires parsed datetime inputs\"\n",
    ),
    (
        "[2024,0,1,0,0,0,0,nan]",
        "mktime",
        "\"mktime requires parsed datetime inputs\"\n",
    ),
    ("[2024,0,1,0,0,0,0,0,nan]", "mktime", "1704067200\n"),
    (
        "nan",
        "gmtime | mktime",
        "\"mktime requires parsed datetime inputs\"\n",
    ),
    (
        "[2024,0,1,0,0,0,nan,0]",
        "strftime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"strftime/1 requires parsed datetime inputs\"\n",
    ),
    (
        "[2024,0,1,0,0,0,0,nan]",
        "strflocaltime(\"%Y-%m-%dT%H:%M:%S\")",
        "\"strflocaltime/1 requires parsed datetime inputs\"\n",
    ),
    (
        "[nan,0,1,0,0,0,0,0]",
        "todate",
        "\"strftime/1 requires parsed datetime inputs\"\n",
    ),
    (
        "nan",
        "strftime(\"%c\")",
        "\"strftime/1 requires parsed datetime inputs\"\n",
    ),
    (
        "nan",
        "strflocaltime(\"%Y\")",
        "\"strflocaltime/1 requires parsed datetime inputs\"\n",
    ),
    (
        "nan",
        "todate",
        "\"strftime/1 requires parsed datetime inputs\"\n",
    ),
    (
        "nan",
        "todateiso8601",
        "\"strftime/1 requires parsed datetime inputs\"\n",
    ),
    ("nan", "gmtime", "[1970,0,1,0,0,null,4,0]\n"),
    ("[2024,0,1]", "mktime", "1704067200\n"),
    ("[2024,0,15,10,30]", "mktime", "1705314600\n"),
    ("[2024,0,15,10,30,5]", "mktime", "1705314605\n"),
    ("[2024,0,15,10,30,5,1]", "mktime", "1705314605\n"),
    ("[2024]", "mktime", "1703980800\n"),
    ("[1970,0,1,0,0,0]", "mktime", "0\n"),
    ("[2024,0,1,0,0,59.9]", "mktime", "1704067259\n"),
    ("[2024.7,0.9,1.9,0,0,0]", "mktime", "1704067200\n"),
    (
        "[2024,2,15]",
        "strftime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"2024-03-15T00:00:00Z\"\n",
    ),
    (
        "[2024,0,15,10,30,5,1]",
        "strftime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"2024-01-15T10:30:05Z\"\n",
    ),
    (
        "[2024,2,15,1,2]",
        "strflocaltime(\"%Y-%m-%dT%H:%M:%S\")",
        "\"2024-03-15T01:02:00\"\n",
    ),
    ("[2024,0,1]", "todate", "\"2024-01-01T00:00:00Z\"\n"),
    (
        "[2024,0,15,10,30,5]",
        "todateiso8601",
        "\"2024-01-15T10:30:05Z\"\n",
    ),
    (
        "[\"a\",0,1]",
        "mktime",
        "\"mktime requires parsed datetime inputs\"\n",
    ),
    (
        "[2024,null]",
        "mktime",
        "\"mktime requires parsed datetime inputs\"\n",
    ),
    (
        "[2024,0,1,\"x\"]",
        "strftime(\"%Y\")",
        "\"strftime/1 requires parsed datetime inputs\"\n",
    ),
    ("[1e20,0,1,0,0,0,0,0]", "mktime", "67768036160140800\n"),
    (
        "[2147485548,0,1,0,0,0,0,0]",
        "mktime",
        "67768036160140800\n",
    ),
    ("[1E1000,0,1]", "mktime", "67768036160140800\n"),
    (
        "[1e20,0,1,0,0,0,0,0]",
        "strftime(\"%Y\")",
        "\"2147485547\"\n",
    ),
    (
        "[-1e20,0,1,0,0,0,0,0]",
        "strftime(\"%Y\")",
        "\"-2147481748\"\n",
    ),
    (
        "[2147483647,0,1,0,0,0,0,0]",
        "strftime(\"%Y\")",
        "\"2147483647\"\n",
    ),
    ("[1899.5,0,1,0,0,0,0,0]", "strftime(\"%Y\")", "\"1900\"\n"),
    ("[-1899.5,0,1,0,0,0,0,0]", "strftime(\"%Y\")", "\"-1899\"\n"),
    (
        "[1e20,0,1,0,0,0,0,0]",
        "strflocaltime(\"%Y\")",
        "\"2147485547\"\n",
    ),
];

#[test]
fn broken_down_time_is_read_as_jq_1_8_2_reads_it_7532() -> Result<()> {
    for &(value, filter, want) in ROWS {
        let program = format!("try ({value} | {filter}) catch .");
        let (output, code) = spawn_with_signal_retry(
            || {
                let mut command = Command::new(succinctly_bin());
                command.env("TZ", "UTC").args(["jq", "-nc", &program]);
                command
            },
            None,
        )?;
        let stdout = String::from_utf8(output.stdout)?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{program}");
    }
    Ok(())
}
