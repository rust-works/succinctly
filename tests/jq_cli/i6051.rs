//! #6051 made `succinctly yq` print a `-0` expression literal as `0` in JSON. jq 1.7.1 kept the
//! sign there; since #7453 `succinctly jq` negates a literal as jq 1.8 does (jqlang/jq#3242), and
//! `decNumberMinus` of `0` is `0`, so jq 1.8.2 and `succinctly jq` print `0` too. A document `-0`
//! still keeps its sign in jq mode, which pins that the yq rule (`yq_json_number_text`, shared
//! with the JSON writers) stays out of jq mode.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_negative_zero_literal_stays_negative_zero_in_jq_json() -> Result<()> {
    for (program, want) in [("-0", "0"), ("[-0]", "[0]"), ("{\"a\": -0}", "{\"a\":0}")] {
        let (stdout, _stderr, code) = run_jq_full(&["-nc", program], None)?;
        assert_eq!(code, 0, "`{program}` failed");
        assert_eq!(stdout.trim(), want, "`{program}`");
    }
    for (program, want) in [(".", "-0"), ("[.]", "[-0]"), ("{a: .}", "{\"a\":-0}")] {
        let (stdout, _stderr, code) = run_jq_full(&["-c", program], Some("-0"))?;
        assert_eq!(code, 0, "`{program}` failed");
        assert_eq!(stdout.trim(), want, "`{program}` on -0");
    }
    Ok(())
}
