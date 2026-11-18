//! #7378: `tojson` of a value the program built very deep answered at most depths and was refused
//! with `nesting depth exceeds limit of the native stack` over a band of them, in a build with
//! debug assertions only. The re-index bridge hands such a value back whole, and a debug-only check
//! then walked it a second time, recursively, on the widest frames of any walk: where the bridge
//! still served the value and that walk no longer fitted under the floor, the check refused what
//! the release build answers. The band was 400,000 to 500,000 levels unoptimised and 1,500,000 and
//! up at `opt-level=1` (M5 Max); the release build never had one.

use anyhow::Result;

use crate::run_jq_full;

/// jq prints a value this deep as 256 brackets, `<skipped: too deep>` and 257 closers: 533
/// characters with the string's quotes, at any depth past its print limit.
#[test]
fn tojson_of_a_deep_built_value_answers_at_every_depth_7378() -> Result<()> {
    for levels in [300_000, 400_000, 500_000, 600_000, 1_500_000] {
        let filter = format!("(reduce range({levels}) as $i ([]; .[0] = .)) | tojson | length");
        let (stdout, stderr, code) = run_jq_full(&["-nc", &filter], None)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            ("533\n", "", 0),
            "{levels} levels"
        );
    }
    Ok(())
}
