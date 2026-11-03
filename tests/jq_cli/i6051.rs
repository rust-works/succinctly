//! #6051 made `succinctly yq` print a `-0` expression literal as `0` in JSON. jq keeps the sign:
//! `/usr/bin/jq` 1.7.1 prints `-0` for `-0` and `[-0]`, and this pins that the yq rule
//! (`yq_json_number_text`, shared with the JSON writers) stays out of jq mode.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_negative_zero_literal_stays_negative_zero_in_jq_json() -> Result<()> {
    for (program, want) in [
        ("-0", "-0"),
        ("[-0]", "[-0]"),
        ("{\"a\": -0}", "{\"a\":-0}"),
    ] {
        let (stdout, _stderr, code) = run_jq_full(&["-nc", program], None)?;
        assert_eq!(code, 0, "`{program}` failed");
        assert_eq!(stdout.trim(), want, "`{program}`");
    }
    Ok(())
}
