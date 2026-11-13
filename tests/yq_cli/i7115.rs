//! #7115: `-o=json` of a scalar forced to `!!float` whose text Go's `strconv.ParseFloat` reads as
//! a non-finite or out-of-range number fails as yq's JSON encoder does, instead of writing the text
//! as a string. A `!!float` whose text `ParseFloat` cannot read at all (`abc`, an empty one, `0x10`, `~`) is its syntax error too. `ParseFloat` takes `inf`, `infinity` and `nan` in any case (`+Inf`/`-Inf` follow the
//! sign, `NaN` has none), so `!!float inf` is `+Inf` and `encoding/json` refuses it; a signed `nan`
//! is a syntax error of `ParseFloat`, and `1e999` its range error. Like a plain `.inf`
//! ([`i6867`](super::i6867)) it is wrapped once per node from the scalar up to the root. Every row
//! was captured from yq v4.53.3. The streamed route has written what comes before the scalar when
//! the failure arrives (yq writes none of it, `limitations.md`), so the failing rows pin the
//! diagnostic and the exit code and leave stdout alone.

use crate::run_yq_stdin_with_stderr;

const WRAP: &str = "json: error calling MarshalJSON for type *yqlib.CandidateNode: ";

/// (input, filter, wraps, cause).
#[rustfmt::skip]
const FAILING: &[(&str, &str, usize, &str)] = &[
    ("a: !!float inf\n", ".", 2, "json: unsupported value: +Inf"),
    ("a: !!float inf\n", ".a", 1, "json: unsupported value: +Inf"),
    ("a: !!float inf\n", "[.]", 3, "json: unsupported value: +Inf"),
    ("a: !!float -inf\n", ".", 2, "json: unsupported value: -Inf"),
    ("a: !!float -inf\n", ".a", 1, "json: unsupported value: -Inf"),
    ("a: !!float -inf\n", "[.]", 3, "json: unsupported value: -Inf"),
    ("a: !!float +inf\n", ".", 2, "json: unsupported value: +Inf"),
    ("a: !!float +inf\n", ".a", 1, "json: unsupported value: +Inf"),
    ("a: !!float +inf\n", "[.]", 3, "json: unsupported value: +Inf"),
    ("a: !!float Inf\n", ".", 2, "json: unsupported value: +Inf"),
    ("a: !!float Inf\n", ".a", 1, "json: unsupported value: +Inf"),
    ("a: !!float Inf\n", "[.]", 3, "json: unsupported value: +Inf"),
    ("a: !!float INF\n", ".", 2, "json: unsupported value: +Inf"),
    ("a: !!float INF\n", ".a", 1, "json: unsupported value: +Inf"),
    ("a: !!float INF\n", "[.]", 3, "json: unsupported value: +Inf"),
    ("a: !!float infinity\n", ".", 2, "json: unsupported value: +Inf"),
    ("a: !!float infinity\n", ".a", 1, "json: unsupported value: +Inf"),
    ("a: !!float infinity\n", "[.]", 3, "json: unsupported value: +Inf"),
    ("a: !!float -Infinity\n", ".", 2, "json: unsupported value: -Inf"),
    ("a: !!float -Infinity\n", ".a", 1, "json: unsupported value: -Inf"),
    ("a: !!float -Infinity\n", "[.]", 3, "json: unsupported value: -Inf"),
    ("a: !!float nan\n", ".", 2, "json: unsupported value: NaN"),
    ("a: !!float nan\n", ".a", 1, "json: unsupported value: NaN"),
    ("a: !!float nan\n", "[.]", 3, "json: unsupported value: NaN"),
    ("a: !!float NaN\n", ".", 2, "json: unsupported value: NaN"),
    ("a: !!float NaN\n", ".a", 1, "json: unsupported value: NaN"),
    ("a: !!float NaN\n", "[.]", 3, "json: unsupported value: NaN"),
    ("a: !!float NAN\n", ".", 2, "json: unsupported value: NaN"),
    ("a: !!float NAN\n", ".a", 1, "json: unsupported value: NaN"),
    ("a: !!float NAN\n", "[.]", 3, "json: unsupported value: NaN"),
    ("a: !!float +nan\n", ".", 2, "strconv.ParseFloat: parsing \"+nan\": invalid syntax"),
    ("a: !!float +nan\n", ".a", 1, "strconv.ParseFloat: parsing \"+nan\": invalid syntax"),
    ("a: !!float +nan\n", "[.]", 3, "strconv.ParseFloat: parsing \"+nan\": invalid syntax"),
    ("a: !!float -nan\n", ".", 2, "strconv.ParseFloat: parsing \"-nan\": invalid syntax"),
    ("a: !!float -nan\n", ".a", 1, "strconv.ParseFloat: parsing \"-nan\": invalid syntax"),
    ("a: !!float -nan\n", "[.]", 3, "strconv.ParseFloat: parsing \"-nan\": invalid syntax"),
    ("a: !!float 1e999\n", ".", 2, "strconv.ParseFloat: parsing \"1e999\": value out of range"),
    ("a: !!float 1e999\n", ".a", 1, "strconv.ParseFloat: parsing \"1e999\": value out of range"),
    ("a: !!float 1e999\n", "[.]", 3, "strconv.ParseFloat: parsing \"1e999\": value out of range"),
    ("a: !!float -1e999\n", ".", 2, "strconv.ParseFloat: parsing \"-1e999\": value out of range"),
    ("a: !!float -1e999\n", ".a", 1, "strconv.ParseFloat: parsing \"-1e999\": value out of range"),
    ("a: !!float -1e999\n", "[.]", 3, "strconv.ParseFloat: parsing \"-1e999\": value out of range"),
    ("a: !!float 1e309\n", ".", 2, "strconv.ParseFloat: parsing \"1e309\": value out of range"),
    ("a: !!float 1e309\n", ".a", 1, "strconv.ParseFloat: parsing \"1e309\": value out of range"),
    ("a: !!float 1e309\n", "[.]", 3, "strconv.ParseFloat: parsing \"1e309\": value out of range"),
    ("a: [!!float inf]\n", ".", 3, "json: unsupported value: +Inf"),
    ("a: {b: !!float nan}\n", ".", 3, "json: unsupported value: NaN"),
    ("!!float inf\n", ".", 1, "json: unsupported value: +Inf"),
    ("a: !!float \"inf\"\n", ".", 2, "json: unsupported value: +Inf"),
    ("a: !!float inf # c\n", ".", 2, "json: unsupported value: +Inf"),
    ("- !!float inf\n", ".", 2, "json: unsupported value: +Inf"),
    ("a: !!float abc\n", ".", 2, "strconv.ParseFloat: parsing \"abc\": invalid syntax"),
    ("a: !!float \"\"\n", ".", 2, "strconv.ParseFloat: parsing \"\": invalid syntax"),
    ("a: !!float 1.2.3\n", ".", 2, "strconv.ParseFloat: parsing \"1.2.3\": invalid syntax"),
    ("a: !!float 12abc\n", ".", 2, "strconv.ParseFloat: parsing \"12abc\": invalid syntax"),
    ("a: !!float true\n", ".", 2, "strconv.ParseFloat: parsing \"true\": invalid syntax"),
    ("a: !!float ~\n", ".", 2, "strconv.ParseFloat: parsing \"~\": invalid syntax"),
    ("a: !!float null\n", ".", 2, "strconv.ParseFloat: parsing \"null\": invalid syntax"),
    ("a: !!float 0x10\n", ".", 2, "strconv.ParseFloat: parsing \"0x10\": invalid syntax"),
    ("a: !!float 0o7\n", ".", 2, "strconv.ParseFloat: parsing \"0o7\": invalid syntax"),
    ("a: !!float 0b11\n", ".", 2, "strconv.ParseFloat: parsing \"0b11\": invalid syntax"),
    ("a: !!float 1_0e999\n", ".", 2, "strconv.ParseFloat: parsing \"1_0e999\": value out of range"),
    ("a: !!float 1__0\n", ".", 2, "strconv.ParseFloat: parsing \"1__0\": invalid syntax"),
    ("a: !!float _1\n", ".", 2, "strconv.ParseFloat: parsing \"_1\": invalid syntax"),
    ("a: !!float 1e\n", ".", 2, "strconv.ParseFloat: parsing \"1e\": invalid syntax"),
    ("a: !!float .\n", ".", 2, "strconv.ParseFloat: parsing \".\": invalid syntax"),
    ("a: !!float +\n", ".", 2, "strconv.ParseFloat: parsing \"+\": invalid syntax"),
    ("a: !!float infinit\n", ".", 2, "strconv.ParseFloat: parsing \"infinit\": invalid syntax"),
    ("a: !!float \" inf\"\n", ".", 2, "strconv.ParseFloat: parsing \" inf\": invalid syntax"),
    ("a: !!float \"inf \"\n", ".", 2, "strconv.ParseFloat: parsing \"inf \": invalid syntax"),
    ("a: !!float |\n  inf\n", ".", 2, "strconv.ParseFloat: parsing \"inf\\n\": invalid syntax"),
    ("a: !!float >\n  nan\n", ".", 2, "strconv.ParseFloat: parsing \"nan\\n\": invalid syntax"),
];

/// (input, filter, stdout): what stays as it was.
#[rustfmt::skip]
const UNCHANGED: &[(&str, &str, &str)] = &[
    ("a: !!float 5\n", ".", "{\"a\":5.0}\n"),
    ("a: !!float 1e308\n", ".", "{\"a\":1e308}\n"),
    ("a: !!float 1.5e3\n", ".a", "1.5e3\n"),
    ("a: !!str inf\n", ".", "{\"a\":\"inf\"}\n"),
    ("a: inf\n", ".a", "\"inf\"\n"),
    ("a: !!str nan\n", "[.]", "[{\"a\":\"nan\"}]\n"),
    ("a: !!float 1_0\n", ".", "{\"a\":10.0}\n"),
    ("a: !!float .5\n", ".", "{\"a\":0.5}\n"),
    ("a: !!float 5.\n", ".", "{\"a\":5.0}\n"),
    ("a: !!float +.5\n", ".", "{\"a\":0.5}\n"),
    ("a: !!float -1.5E+3\n", ".", "{\"a\":-1.5E+3}\n"),
    ("a: !!float 007\n", ".", "{\"a\":7.0}\n"),
    ("a: !!float 1_000.5_5\n", ".", "{\"a\":1000.55}\n"),
];

#[test]
fn a_tagged_non_finite_float_fails_in_the_json_writer_7115() {
    for &(input, filter, wraps, cause) in FAILING {
        let (_, stderr, status) =
            run_yq_stdin_with_stderr(filter, input, &["-o", "json", "-I0"]).expect("run");
        let expected = format!("Error: {}{cause}", WRAP.repeat(wraps));
        assert_eq!(
            stderr.lines().next().unwrap_or(""),
            expected,
            "{filter:?} on {input:?}"
        );
        assert_eq!(status, 1, "{filter:?} on {input:?}");
    }
}

#[test]
fn a_finite_or_untagged_float_is_written_as_before_7115() {
    for &(input, filter, stdout) in UNCHANGED {
        let (out, _, status) =
            run_yq_stdin_with_stderr(filter, input, &["-o", "json", "-I0"]).expect("run");
        assert_eq!(
            (out.as_str(), status),
            (stdout, 0),
            "{filter:?} on {input:?}"
        );
    }
}
