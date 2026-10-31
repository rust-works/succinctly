//! #5594: `tojson` writes a document number into the output buffer, copying a literal jq would
//! print unchanged and formatting only the others, with no `String` per number. The text must not
//! move on either side of that split, through the plain printer, an assignment (a materialized
//! document) or `--slurp` (the input bridge). Every row was captured from jq 1.7.1
//! (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

/// Canonical spellings (copied) beside every respelled kind: a trailing zero, an exponent, a
/// negative zero, a decimal decNumber writes in scientific notation, and an overflowing literal.
const DOCUMENT: &str = r#"{"a":[0,7,-12,25,0.5,-0.25,0.50,1.0,100000000000000000000,1e2,1E+2,-0,0.0000001,12.345,1e400]}"#;

/// `(arguments, stdin, expected stdout)`.
type Row = (&'static [&'static str], &'static str, &'static str);

const ROWS: &[Row] = &[
    (
        &["-c", "tojson"],
        DOCUMENT,
        r#""{\"a\":[0,7,-12,25,0.5,-0.25,0.50,1.0,100000000000000000000,1E+2,1E+2,-0,1E-7,12.345,1E+400]}""#,
    ),
    (
        &["-c", ".a | map(tojson)"],
        DOCUMENT,
        r#"["0","7","-12","25","0.5","-0.25","0.50","1.0","100000000000000000000","1E+2","1E+2","-0","1E-7","12.345","1E+400"]"#,
    ),
    (
        &["-c", ".a[0] = 1 | tojson"],
        DOCUMENT,
        r#""{\"a\":[1,7,-12,25,0.5,-0.25,0.50,1.0,100000000000000000000,1E+2,1E+2,-0,1E-7,12.345,1E+400]}""#,
    ),
    (
        &["-c", "-s", ".[0] | tojson"],
        DOCUMENT,
        r#""{\"a\":[0,7,-12,25,0.5,-0.25,0.50,1.0,100000000000000000000,1E+2,1E+2,-0,1E-7,12.345,1E+400]}""#,
    ),
];

#[test]
fn test_tojson_writes_document_numbers_like_jq_5594() -> Result<()> {
    for (args, stdin, expected) in ROWS {
        let (stdout, stderr, status) = run_jq_full(args, Some(stdin))?;
        assert_eq!(
            (stdout.trim_end(), status),
            (*expected, 0),
            "{args:?}: stderr {stderr}"
        );
    }
    Ok(())
}
