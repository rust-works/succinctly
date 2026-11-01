//! #5325: yq-mode `fromjson` skips up to two byte-order marks (see `tests/yq_cli/i5325.rs`); jq
//! mode still skips exactly one (#5312). Captured from jq 1.7.1 (`/usr/bin/jq`); stderr wording is
//! not compared.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_jq_fromjson_still_skips_exactly_one_byte_order_mark_5325() -> Result<()> {
    let mark = "\u{feff}";
    for (marks, expected, status) in [(1, "[1]\n", 0), (2, "", 5)] {
        let program = format!("\"{}[1]\" | fromjson", mark.repeat(marks));
        let (stdout, _stderr, code) = run_jq_full(&["-nc", &program], None)?;
        assert_eq!(
            (stdout.as_str(), code),
            (expected, status),
            "{marks} mark(s)"
        );
    }
    Ok(())
}
