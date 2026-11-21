//! #7440: `-e` makes `halt_error(N)` for a negative N exit with abs(N) (mod 256), as in jq 1.7.1.
//!
//! jq's `jq_exit_with_status` is `exit(abs(r))` and the plain `jq_exit` clamps a negative status
//! to 0, so the sign matters only with `-e`/`--exit-status`: `halt_error(-300)` exits 44 with it
//! and 0 without. The code is a C `int` made from the number, so a fraction truncates toward
//! zero, an out-of-range value saturates, and `abs(INT_MIN)` is `INT_MIN` again (low byte 0).
//! Every row is captured from `/usr/bin/jq` 1.7.1 (`jq -n [-e] FILTER`, exit status).

use anyhow::Result;

use crate::run_jq_full;

/// `(filter, exit status without -e, exit status with -e)`.
const ROWS: &[(&str, i32, i32)] = &[
    ("halt_error(-1)", 0, 1),
    ("halt_error(-2)", 0, 2),
    ("halt_error(-255)", 0, 255),
    ("halt_error(-256)", 0, 0),
    ("halt_error(-257)", 0, 1),
    ("halt_error(-300)", 0, 44),
    ("halt_error(-100000)", 0, 160),
    ("halt_error(-65537)", 0, 1),
    ("halt_error(-1.5)", 0, 1),
    ("halt_error(-0.5)", 0, 0),
    ("halt_error(-2.9)", 0, 2),
    ("halt_error(-1e10)", 0, 0),
    ("halt_error(-2147483648)", 0, 0),
    ("halt_error(-2147483649)", 0, 0),
    ("halt_error(-infinite)", 0, 0),
    ("halt_error(nan)", 0, 0),
    ("halt_error(0)", 0, 0),
    ("halt_error(7)", 7, 7),
    ("halt_error(300)", 44, 44),
    ("halt_error(2.7)", 2, 2),
    ("halt_error(5-8)", 0, 3),
    ("halt_error(-(1,2))", 0, 1),
    ("halt_error(-3,-4)", 0, 3),
    (r#""x"|halt_error(-3)"#, 0, 3),
    ("1|halt_error(-3)", 0, 3),
    ("(1,2)|halt_error(-3)", 0, 3),
];

#[test]
fn test_7440_negative_halt_error_code_with_and_without_e() -> Result<()> {
    for &(filter, plain, with_e) in ROWS {
        for (flags, want) in [
            (&["-n"][..], plain),
            (&["-n", "-e"][..], with_e),
            (&["-en"][..], with_e),
            (&["-n", "--exit-status"][..], with_e),
        ] {
            let mut args = flags.to_vec();
            args.push(filter);
            let (stdout, stderr, code) = run_jq_full(&args, None)?;
            assert_eq!(
                code, want,
                "{flags:?} {filter}: stdout {stdout:?} stderr {stderr:?}"
            );
        }
    }
    Ok(())
}

/// `i32::MAX` is the saturated code of `-2147483647`'s abs; Unix keeps its low byte (255) as the
/// observed status where Windows would not (the same gate as the positive rows of
/// `test_halt_error_positive_and_special_exit_codes_unaffected`).
#[cfg(unix)]
#[test]
fn test_7440_negative_halt_error_code_saturating_rows() -> Result<()> {
    for (filter, plain, with_e) in [
        ("halt_error(-2147483647)", 0, 255),
        ("halt_error(4294967296)", 255, 255),
    ] {
        for (flags, want) in [(&["-n"][..], plain), (&["-n", "-e"][..], with_e)] {
            let mut args = flags.to_vec();
            args.push(filter);
            let (_, stderr, code) = run_jq_full(&args, None)?;
            assert_eq!(code, want, "{flags:?} {filter}: stderr {stderr:?}");
        }
    }
    Ok(())
}

/// The code applies to a run over input as well, and the message still goes to stderr.
#[test]
fn test_7440_negative_halt_error_code_over_stdin_input() -> Result<()> {
    for (flags, want) in [(&[][..], 0), (&["-e"][..], 3)] {
        let mut args = flags.to_vec();
        args.extend(["if . == 2 then halt_error(-3) else . end"]);
        let (stdout, stderr, code) = run_jq_full(&args, Some("1 2 3"))?;
        assert_eq!(stdout, "1\n", "{flags:?}");
        assert_eq!(stderr, "2\n", "{flags:?}");
        assert_eq!(code, want, "{flags:?}");
    }
    // A file jq cannot open outranks the halt's code, with or without -e.
    for flags in [&["-e"][..], &[][..]] {
        let mut args = flags.to_vec();
        args.extend(["halt_error(-3)", "/nonexistent-7440.json"]);
        let (_, _, code) = run_jq_full(&args, None)?;
        assert_eq!(code, 2, "{flags:?}");
    }
    Ok(())
}
