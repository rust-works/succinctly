//! #7141 (the JSON slice): go-yaml types an unsigned integer it reads with `ParseUint` (base 0: decimal, `_`
//! separators, `0x`, `0o`, `0b`, a leading `0` for octal) as `!!int` up to 2^64 - 1, and yq's JSON encoder then calls
//! `ParseInt` on its text, which refuses 2^63 and above: `-o json` of such a scalar is
//! `strconv.ParseInt: parsing "9223372036854775808": value out of range` (the digits alone for `0x` and `0o`,
//! `invalid syntax` with the prefix kept for `0b` and a capital `0O`), wrapped once per node up to the root.
//! `succinctly yq` printed the decimal as a rounded float (`9223372036854776000.0`) and a `0x` spelling as a string.
//! A sign, `.0`, 2^64 and above, and the YAML and props encoders are not affected, as in yq.
//!
//! Every row was captured from yq v4.53.3. What still differs, listed in `limitations.md`: a value the filter builds
//! (`[.a]`), arithmetic, `tag` and `type` (`!!int` in yq, `!!float` here), and the owned routes.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

/// `(document, filter, yq's error line)` under `-o json -I=0`.
const ERRS: &[(&str, &str, &str)] = &[
    ("a: 9223372036854775808\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"9223372036854775808\": value out of range"),
    ("- 9223372036854775808\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"9223372036854775808\": value out of range"),
    ("k:\n  - 9223372036854775808\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"9223372036854775808\": value out of range"),
    ("a: 9223372036854775808\n", ".a", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"9223372036854775808\": value out of range"),
    ("a: 18446744073709551615\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"18446744073709551615\": value out of range"),
    ("- 18446744073709551615\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"18446744073709551615\": value out of range"),
    ("k:\n  - 18446744073709551615\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"18446744073709551615\": value out of range"),
    ("a: 18446744073709551615\n", ".a", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"18446744073709551615\": value out of range"),
    ("a: 9_223_372_036_854_775_808\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"9223372036854775808\": value out of range"),
    ("- 9_223_372_036_854_775_808\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"9223372036854775808\": value out of range"),
    ("k:\n  - 9_223_372_036_854_775_808\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"9223372036854775808\": value out of range"),
    ("a: 9_223_372_036_854_775_808\n", ".a", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"9223372036854775808\": value out of range"),
    ("a: 0x8000000000000000\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"8000000000000000\": value out of range"),
    ("- 0x8000000000000000\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"8000000000000000\": value out of range"),
    ("k:\n  - 0x8000000000000000\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"8000000000000000\": value out of range"),
    ("a: 0x8000000000000000\n", ".a", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"8000000000000000\": value out of range"),
    ("a: 0xFFFFFFFFFFFFFFFF\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"FFFFFFFFFFFFFFFF\": value out of range"),
    ("- 0xFFFFFFFFFFFFFFFF\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"FFFFFFFFFFFFFFFF\": value out of range"),
    ("k:\n  - 0xFFFFFFFFFFFFFFFF\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"FFFFFFFFFFFFFFFF\": value out of range"),
    ("a: 0xFFFFFFFFFFFFFFFF\n", ".a", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"FFFFFFFFFFFFFFFF\": value out of range"),
    ("a: 0XFfFfFfFfFfFfFfFf\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"FfFfFfFfFfFfFfFf\": value out of range"),
    ("- 0XFfFfFfFfFfFfFfFf\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"FfFfFfFfFfFfFfFf\": value out of range"),
    ("k:\n  - 0XFfFfFfFfFfFfFfFf\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"FfFfFfFfFfFfFfFf\": value out of range"),
    ("a: 0XFfFfFfFfFfFfFfFf\n", ".a", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"FfFfFfFfFfFfFfFf\": value out of range"),
    ("a: 0x8_000000000000000\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"8000000000000000\": value out of range"),
    ("- 0x8_000000000000000\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"8000000000000000\": value out of range"),
    ("k:\n  - 0x8_000000000000000\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"8000000000000000\": value out of range"),
    ("a: 0x8_000000000000000\n", ".a", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"8000000000000000\": value out of range"),
    ("a: 0o1000000000000000000000\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"1000000000000000000000\": value out of range"),
    ("- 0o1000000000000000000000\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"1000000000000000000000\": value out of range"),
    ("k:\n  - 0o1000000000000000000000\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"1000000000000000000000\": value out of range"),
    ("a: 0o1000000000000000000000\n", ".a", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"1000000000000000000000\": value out of range"),
    ("a: 0O1777777777777777777777\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"0O1777777777777777777777\": invalid syntax"),
    ("- 0O1777777777777777777777\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"0O1777777777777777777777\": invalid syntax"),
    ("k:\n  - 0O1777777777777777777777\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"0O1777777777777777777777\": invalid syntax"),
    ("a: 0O1777777777777777777777\n", ".a", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"0O1777777777777777777777\": invalid syntax"),
    ("a: 0b1000000000000000000000000000000000000000000000000000000000000000\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"0b1000000000000000000000000000000000000000000000000000000000000000\": invalid syntax"),
    ("- 0b1000000000000000000000000000000000000000000000000000000000000000\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"0b1000000000000000000000000000000000000000000000000000000000000000\": invalid syntax"),
    ("k:\n  - 0b1000000000000000000000000000000000000000000000000000000000000000\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"0b1000000000000000000000000000000000000000000000000000000000000000\": invalid syntax"),
    ("a: 0b1000000000000000000000000000000000000000000000000000000000000000\n", ".a", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"0b1000000000000000000000000000000000000000000000000000000000000000\": invalid syntax"),
    ("a: 0B1111111111111111111111111111111111111111111111111111111111111111\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"0B1111111111111111111111111111111111111111111111111111111111111111\": invalid syntax"),
    ("- 0B1111111111111111111111111111111111111111111111111111111111111111\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"0B1111111111111111111111111111111111111111111111111111111111111111\": invalid syntax"),
    ("k:\n  - 0B1111111111111111111111111111111111111111111111111111111111111111\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"0B1111111111111111111111111111111111111111111111111111111111111111\": invalid syntax"),
    ("a: 0B1111111111111111111111111111111111111111111111111111111111111111\n", ".a", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"0B1111111111111111111111111111111111111111111111111111111111111111\": invalid syntax"),
    ("a: 01777777777777777777777\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"01777777777777777777777\": value out of range"),
    ("- 01777777777777777777777\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"01777777777777777777777\": value out of range"),
    ("k:\n  - 01777777777777777777777\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"01777777777777777777777\": value out of range"),
    ("a: 01777777777777777777777\n", ".a", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"01777777777777777777777\": value out of range"),
    ("a: 12345678901234567890\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"12345678901234567890\": value out of range"),
    ("- 12345678901234567890\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"12345678901234567890\": value out of range"),
    ("k:\n  - 12345678901234567890\n", ".", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"12345678901234567890\": value out of range"),
    ("a: 12345678901234567890\n", ".a", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseInt: parsing \"12345678901234567890\": value out of range"),
];

/// `(document, flags, stdout)`: what is not an error.
const OKS: &[(&str, &str, &str)] = &[
    (
        "a: 9223372036854775807\n",
        "-o json -I=0 .",
        "{\"a\":9223372036854775807}\n",
    ),
    (
        "a: 9223372036854775807\n",
        "-o yaml .",
        "a: 9223372036854775807\n",
    ),
    (
        "a: 18446744073709551616\n",
        "-o json -I=0 .",
        "{\"a\":18446744073709552000.0}\n",
    ),
    (
        "a: 18446744073709551616\n",
        "-o yaml .",
        "a: 18446744073709551616\n",
    ),
    (
        "a: +9223372036854775808\n",
        "-o json -I=0 .",
        "{\"a\":9223372036854776000.0}\n",
    ),
    (
        "a: +9223372036854775808\n",
        "-o yaml .",
        "a: +9223372036854775808\n",
    ),
    (
        "a: 0x10000000000000000\n",
        "-o json -I=0 .",
        "{\"a\":\"0x10000000000000000\"}\n",
    ),
    (
        "a: 0x10000000000000000\n",
        "-o yaml .",
        "a: 0x10000000000000000\n",
    ),
    (
        "a: 0x7FFFFFFFFFFFFFFF\n",
        "-o json -I=0 .",
        "{\"a\":9223372036854775807}\n",
    ),
    (
        "a: 0x7FFFFFFFFFFFFFFF\n",
        "-o yaml .",
        "a: 0x7FFFFFFFFFFFFFFF\n",
    ),
    (
        "a: -9223372036854775809\n",
        "-o json -I=0 .",
        "{\"a\":-9223372036854776000.0}\n",
    ),
    (
        "a: -9223372036854775809\n",
        "-o yaml .",
        "a: -9223372036854775809\n",
    ),
    (
        "a: 9223372036854775808.0\n",
        "-o json -I=0 .",
        "{\"a\":9223372036854775808.0}\n",
    ),
    (
        "a: 9223372036854775808.0\n",
        "-o yaml .",
        "a: 9223372036854775808.0\n",
    ),
    (
        "a: 1234567890123456789\n",
        "-o json -I=0 .",
        "{\"a\":1234567890123456789}\n",
    ),
    (
        "a: 1234567890123456789\n",
        "-o yaml .",
        "a: 1234567890123456789\n",
    ),
    ("a: 0x\n", "-o json -I=0 .", "{\"a\":\"0x\"}\n"),
    ("a: 0x\n", "-o yaml .", "a: 0x\n"),
    (
        "a: 0b2000000000000000000000000000000000000000000000000000000000000000\n",
        "-o json -I=0 .",
        "{\"a\":\"0b2000000000000000000000000000000000000000000000000000000000000000\"}\n",
    ),
    (
        "a: 0b2000000000000000000000000000000000000000000000000000000000000000\n",
        "-o yaml .",
        "a: 0b2000000000000000000000000000000000000000000000000000000000000000\n",
    ),
    (
        "a: 9223372036854775808\n",
        "-o yaml .",
        "a: 9223372036854775808\n",
    ),
    (
        "a: 9223372036854775808\n",
        "-o props .",
        "a = 9223372036854775808\n",
    ),
    (
        "a: 0x8000000000000000\n",
        "-o yaml .",
        "a: 0x8000000000000000\n",
    ),
    (
        "a: 0x8000000000000000\n",
        "-o props .",
        "a = 0x8000000000000000\n",
    ),
];

fn error_line(stderr: &str) -> &str {
    stderr.lines().next().unwrap_or("")
}

#[test]
fn an_integer_past_i64_fails_in_json_output_as_in_yq_7141() -> Result<()> {
    for &(doc, filter, want) in ERRS {
        let (_, stderr, code) = run_yq_stdin_with_stderr(filter, doc, &["-o", "json", "-I=0"])?;
        assert_eq!(code, 1, "{doc:?} {filter}");
        assert_eq!(error_line(&stderr), want, "{doc:?} {filter}");
    }
    Ok(())
}

#[test]
fn an_integer_that_fits_a_float_or_a_string_or_is_not_json_is_unchanged_7141() -> Result<()> {
    for &(doc, flags, want) in OKS {
        let flags: Vec<&str> = flags.split(' ').filter(|f| *f != ".").collect();
        let (stdout, _, code) = run_yq_stdin_with_stderr(".", doc, &flags)?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{doc:?} {flags:?}");
    }
    Ok(())
}
