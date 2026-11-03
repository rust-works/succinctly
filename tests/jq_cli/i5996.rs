//! #5996: `strptime`'s `%s` reads the whole digit run, so leading zeros count for nothing: the
//! 19-digit limit is on the significant digits (`"00000000000000000001"` is the epoch plus one
//! second, where the first 19 characters were read and the `1` was left over). Every answer below was
//! captured from jq 1.7.1 under `TZ=UTC`, first six fields.

use std::process::Command;

use anyhow::Result;

use crate::{spawn_with_signal_retry, succinctly_bin};

fn run_in_zone(zone: &str, input: &str, fmt: &str) -> Result<(String, i32)> {
    let (output, code) = spawn_with_signal_retry(
        || {
            let mut command = Command::new(succinctly_bin());
            command.env("TZ", zone).args([
                "jq",
                "-nc",
                "--arg",
                "i",
                input,
                "--arg",
                "f",
                fmt,
                "$i|strptime($f)|.[0:6]",
            ]);
            command
        },
        None,
    )?;
    Ok((String::from_utf8(output.stdout)?, code))
}

#[test]
fn leading_zeros_do_not_count_against_the_nineteen_digit_limit() -> Result<()> {
    for (zone, input, fmt, want) in [
        ("UTC", "0000000000000000000", "%s", "[1970,0,1,0,0,0]\n"),
        ("UTC", "000000000000000000001", "%s", "[1970,0,1,0,0,1]\n"),
        (
            "UTC",
            "0000000000000000000000000042",
            "%s",
            "[1970,0,1,0,0,42]\n",
        ),
        (
            "UTC",
            "0000000000000000000000000000",
            "%s",
            "[1970,0,1,0,0,0]\n",
        ),
        (
            "UTC",
            "-000000000000000000001",
            "%s",
            "[1969,11,31,23,59,59]\n",
        ),
        ("UTC", "+0000000000000000000001", "%s", "[1970,0,1,0,0,1]\n"),
        (
            "UTC",
            "  00000000000000000000005",
            "%s",
            "[1970,0,1,0,0,5]\n",
        ),
        (
            "UTC",
            "000000000000000000000042 07",
            "%s %H",
            "[1970,0,1,7,0,42]\n",
        ),
        (
            "UTC",
            "000000000000000000000012x",
            "%sx",
            "[1970,0,1,0,0,12]\n",
        ),
        (
            "UTC",
            "000000000000000000000000000000",
            "%s",
            "[1970,0,1,0,0,0]\n",
        ),
        (
            "UTC",
            "00000000000000000000000009223372036854775808",
            "%s",
            "",
        ),
        ("UTC", "99999999999999999999", "%s", ""),
        ("UTC", "9223372036854775808", "%s", ""),
        (
            "UTC",
            "0000000000000000000001425599621",
            "%s",
            "[2015,2,5,23,53,41]\n",
        ),
        ("UTC", "12 ", "%s", "[1970,0,1,0,0,12]\n"),
        (
            "UTC",
            "00000000000000000000012 ",
            "%s",
            "[1970,0,1,0,0,12]\n",
        ),
    ] {
        let (out, code) = run_in_zone(zone, input, fmt)?;
        if want.is_empty() {
            assert_ne!(code, 0, "{input:?} | strptime({fmt:?}) matched: {out}");
        } else {
            assert_eq!(code, 0, "{input:?} | strptime({fmt:?})");
            assert_eq!(out, want, "{input:?} | strptime({fmt:?})");
        }
    }
    Ok(())
}
