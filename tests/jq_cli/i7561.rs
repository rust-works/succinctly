//! #7561: the exit status of `halt_error(n)` for an `n` of 2^31 or more, or `infinite`.
//!
//! jq 1.7.1 and 1.8.2 convert the argument with an implicit double-to-int conversion
//! (`ret = jv_number_value(exit_code)` in `main.c`), which is undefined behaviour in C for a double
//! outside `int`'s range: it saturates on aarch64, so every such value exits 255, and returns
//! `INT_MIN` on x86_64, whose low byte is 0, so the same values exit 0 there. `succinctly jq`
//! saturates on every platform and so equals jq on aarch64 and differs from it on x86_64 only. This
//! is a recorded divergence (limitations.md), pinned here so that a change does not make the status
//! depend on the CPU, and so that the rows that agree on both architectures keep agreeing.
//!
//! Every expectation below is `succinctly jq`'s on every platform; the in-range rows are also
//! jq 1.7.1's and 1.8.2's on x86_64 and aarch64 alike, and the out-of-range rows are jq's on
//! aarch64 (captured from `/usr/bin/jq` 1.7.1 and `/opt/homebrew/bin/jq` 1.8.2 on an Apple M5 Max;
//! the x86_64 Linux rows are the issue's measurements, jq 1.7.1, not re-run here).

use anyhow::Result;

use crate::run_jq_full;

/// `(argument, exit status)`. The message `x\n` reaches stderr unchanged in every row.
const ROWS: &[(&str, i32)] = &[
    // In range: the argument is truncated toward zero and its low byte is the status, in jq too.
    ("0", 0),
    ("1", 1),
    ("2", 2),
    ("5", 5),
    ("127", 127),
    ("254", 254),
    ("255", 255),
    ("256", 0),
    ("257", 1),
    ("1.5", 1),
    ("0.5", 0),
    ("2147483646", 254),
    ("2147483647", 255),
    // Negative values, `nan` and `-infinite` exit 0 in jq, on both architectures.
    ("-1", 0),
    ("-2", 0),
    ("-255", 0),
    ("-256", 0),
    ("-257", 0),
    ("-2147483648", 0),
    ("-2147483649", 0),
    ("-1e10", 0),
    ("-1e100", 0),
    ("-infinite", 0),
    ("nan", 0),
    // Out of range above: 255 here on every platform; jq's 255 on aarch64, 0 on x86_64 Linux.
    ("2147483648", 255),
    ("2147483649", 255),
    ("4294967295", 255),
    ("4294967296", 255),
    ("4294967297", 255),
    ("3000000000", 255),
    ("1e10", 255),
    ("1e19", 255),
    ("1e100", 255),
    ("infinite", 255),
];

#[test]
fn test_halt_error_exit_status_is_the_same_on_every_platform_7561() -> Result<()> {
    for (arg, want) in ROWS {
        let program = format!("\"x\\n\" | halt_error({arg})");
        let (stdout, stderr, status) = run_jq_full(&["-n", &program], None)?;
        assert_eq!(status, *want, "halt_error({arg}) exit status");
        assert_eq!(stderr, "x\n", "halt_error({arg}) stderr");
        assert_eq!(stdout, "", "halt_error({arg}) stdout");
    }
    Ok(())
}

/// The saturated status survives the other routes a `halt_error` can take: after output, in a
/// `try`, under `-e`, and with an object message (which jq prints as JSON).
#[test]
fn test_halt_error_saturated_status_on_other_routes_7561() -> Result<()> {
    for (program, stdout, stderr, want) in [
        ("1, (\"x\\n\" | halt_error(4294967296))", "1\n", "x\n", 255),
        ("try (\"x\\n\" | halt_error(1e19)) catch 0", "", "x\n", 255),
        ("{\"a\":1} | halt_error(infinite)", "", "{\"a\":1}\n", 255),
    ] {
        let (out, err, status) = run_jq_full(&["-n", program], None)?;
        assert_eq!(
            (out.as_str(), err.as_str(), status),
            (stdout, stderr, want),
            "{program}"
        );
    }
    let (out, err, status) = run_jq_full(&["-n", "-e", "\"x\\n\" | halt_error(2147483648)"], None)?;
    assert_eq!((out.as_str(), err.as_str(), status), ("", "x\n", 255));
    Ok(())
}
