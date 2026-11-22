//! #7730: the computed double `2^-24` printed as `5.960464477539062e-08`, which reads back as the
//! next lower double. It is the exact midpoint of `...062e-08` and `...063e-08`, and the #2542
//! tie correction picked the even digit as jq does on a tie, but `2^-24` is a power of two: the
//! double below it is half as far away as the one above, so only `...063e-08` round-trips, and
//! that is what jq prints. Every row is captured from `/usr/bin/jq` 1.7.1 (macOS; the Linux build
//! agrees), and `/opt/homebrew/bin/jq` 1.8.2 agrees on every row.

use anyhow::Result;

use crate::run_jq_full;

/// (extra flags, input, filter, stdout).
const ROWS: &[(&[&str], &str, &str, &str)] = &[
    (&["-n"], "", "1/16777216", "5.960464477539063e-08\n"),
    (&["-n"], "", "-1/16777216", "-5.960464477539063e-08\n"),
    (
        &["-n"],
        "",
        "1/16777216 | [., (tojson|tonumber == 1/16777216)]",
        "[5.960464477539063e-08,true]\n",
    ),
    (
        &["-n"],
        "",
        "[1/16777216, -128/2147483648]",
        "[5.960464477539063e-08,-5.960464477539063e-08]\n",
    ),
    (
        &["-n"],
        "",
        "1/16777216 | tostring",
        "\"5.960464477539063e-08\"\n",
    ),
    (
        &["-n"],
        "",
        "1/16777216 | tojson",
        "\"5.960464477539063e-08\"\n",
    ),
    (
        &["-n"],
        "",
        "1/16777216 | @text",
        "\"5.960464477539063e-08\"\n",
    ),
    (
        &["-n"],
        "",
        "[1/16777216] | @csv",
        "\"5.960464477539063e-08\"\n",
    ),
    (
        &["-n"],
        "",
        "[1/16777216] | @tsv",
        "\"5.960464477539063e-08\"\n",
    ),
    (
        &["-n"],
        "",
        "{a: (1/16777216)} | @json",
        "\"{\\\"a\\\":5.960464477539063e-08}\"\n",
    ),
    (
        &["-n"],
        "",
        "[16777216 | ., 1/., -1/.] | map(tostring)",
        "[\"16777216\",\"5.960464477539063e-08\",\"-5.960464477539063e-08\"]\n",
    ),
    // A document number made a computed double by arithmetic; `.` alone keeps its literal, in
    // jq's canonical spelling.
    (
        &[],
        "[5.9604644775390625e-08]\n",
        "map(. * 1), map(. / 1 | tostring), .",
        "[5.960464477539063e-08]\n[\"5.960464477539063e-08\"]\n[5.9604644775390625E-8]\n",
    ),
    // The neighbours: 2^-25 is a tie whose lower candidate round-trips (#2542 keeps it), and the
    // rest are not ties.
    (
        &["-n"],
        "",
        "[1/33554432, 1/8388608, 3/16777216, 1/16777216 * 1.5]",
        "[2.9802322387695312e-08,1.1920928955078125e-07,1.7881393432617188e-07,8.940696716308594e-08]\n",
    ),
];

#[test]
fn test_two_to_the_minus_24_prints_text_that_reads_back_7730() -> Result<()> {
    for &(flags, input, filter, want) in ROWS {
        let mut args = vec!["-c"];
        args.extend_from_slice(flags);
        args.push(filter);
        let got = run_jq_full(&args, Some(input))?;
        assert_eq!(
            got,
            (want.to_string(), String::new(), 0),
            "{flags:?} {filter} over {input:?}"
        );
    }
    Ok(())
}

/// The pretty printer and `--seq` take the same digits.
#[test]
fn test_two_to_the_minus_24_pretty_and_seq_7730() -> Result<()> {
    let got = run_jq_full(&["-n", "{a: (1/16777216)}"], None)?;
    assert_eq!(
        got,
        (
            "{\n  \"a\": 5.960464477539063e-08\n}\n".to_string(),
            String::new(),
            0
        )
    );
    let got = run_jq_full(&["-n", "--seq", "1/16777216"], None)?;
    assert_eq!(
        got,
        (
            "\u{1e}5.960464477539063e-08\n".to_string(),
            String::new(),
            0
        )
    );
    Ok(())
}
