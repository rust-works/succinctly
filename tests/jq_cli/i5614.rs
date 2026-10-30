//! #5614: a number literal spelled with an empty fraction (`1.e1`) is the decNumber `1e1`, so jq
//! prints `1E+1` for it as for `1e1`. Every expected byte string was captured from the pinned
//! jq 1.7.1 source build (`scripts/compare-jq-build.sh 1.7.1`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_empty_fraction_literal_keeps_its_decnumber_spelling_5614() -> Result<()> {
    for (program, expected) in [
        ("1.e1", "1E+1\n"),
        ("1.E1", "1E+1\n"),
        ("1.e+1", "1E+1\n"),
        ("1.e-1", "0.1\n"),
        ("1.e0", "1\n"),
        ("10.e1", "1.0E+2\n"),
        ("5.e1", "5E+1\n"),
        ("0.e5", "0E+5\n"),
        ("1.e1000", "1E+1000\n"),
        ("9.e999999999", "9E+999999999\n"),
        (
            "123456789012345678901234567890.e2",
            "1.23456789012345678901234567890E+31\n",
        ),
        ("[1.e1,2.E2]", "[1E+1,2E+2]\n"),
        (r#"{"a":1.e1}"#, "{\"a\":1E+1}\n"),
        ("1.e1|tojson", "\"1E+1\"\n"),
        ("\"\\(1.e1)\"", "\"1E+1\"\n"),
        // Arithmetic, comparison and negation see the value, not the spelling.
        ("1.e1 == 10", "true\n"),
        ("1.e1+0", "10\n"),
        ("3.e2|floor", "300\n"),
        ("-1.e1", "-10\n"),
        ("1.e1|-.", "-10\n"),
        // A trailing dot with no exponent keeps the literal's digits too: the dot is dropped
        // before the validity check, where it used to collapse a wide one to the double.
        ("1.", "1\n"),
        (
            "123456789012345678901234567890.",
            "123456789012345678901234567890\n",
        ),
        ("100000000000000000000.", "100000000000000000000\n"),
        ("9007199254740993.", "9007199254740993\n"),
        ("9007199254740993.|tojson", "\"9007199254740993\"\n"),
        ("9007199254740993.|.+0", "9007199254740992\n"),
        // As an index.
        ("[range(3)]|.[1.e0]", "1\n"),
    ] {
        let (out, err, code) = run_jq_full(&["-nc", program], None)?;
        assert_eq!((out.as_str(), code), (expected, 0), "{program}: {err}");
    }
    Ok(())
}
