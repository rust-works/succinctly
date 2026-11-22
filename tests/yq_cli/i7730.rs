//! #7730: yq mode shares jq mode's tie correction (`correct_shortest_decimal_tiebreak`), and printed
//! the computed double `2^-24` as `5.960464477539062e-08`, which reads back as the next lower
//! double. yq v4.53.3 prints `5.960464477539063e-08` (Go's `strconv` shortest digits); each row
//! is a live capture.

use anyhow::Result;

use crate::run_yq_stdin;

const DOC: &str = "a: 5.9604644775390625e-08\n";

/// (extra flags, filter, expected output).
const ROWS: &[(&[&str], &str, &str)] = &[
    (&[], ".a * 1", "5.960464477539063e-08\n"),
    (&[], ".a / 1", "5.960464477539063e-08\n"),
    (&[], ".a * -1", "-5.960464477539063e-08\n"),
    (&[], "[.a * 1]", "- 5.960464477539063e-08\n"),
    (&[], ".a * 1 | tostring", "5.960464477539063e-08\n"),
    (
        &["-o", "json", "-I0"],
        "[.a * 1]",
        "[5.960464477539063e-08]\n",
    ),
    (
        &["-o", "json", "-I0"],
        ".a * 1 | tostring",
        "\"5.960464477539063e-08\"\n",
    ),
    // The document's own text is kept.
    (&[], ".a", "5.9604644775390625e-08\n"),
];

#[test]
fn test_two_to_the_minus_24_prints_text_that_reads_back_7730() -> Result<()> {
    for &(flags, filter, want) in ROWS {
        let (out, code) = run_yq_stdin(filter, DOC, flags)?;
        assert_eq!((out.as_str(), code), (want, 0), "{flags:?} {filter}");
    }
    // 2^-25 is a tie whose lower candidate round-trips: yq prints the even digit, as before.
    let (out, code) = run_yq_stdin(".a * 1", "a: 2.98023223876953125e-08\n", &[])?;
    assert_eq!((out.as_str(), code), ("2.9802322387695312e-08\n", 0));
    Ok(())
}
