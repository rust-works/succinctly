//! #5782: dropping a value built millions of levels deep used to overflow the native stack
//! (`OwnedValue` had no iterative `Drop`; exit 134). jq 1.7.1 does not answer these programs
//! either at this depth (it segfaults from about 300,000 levels), so there is nothing to diff:
//! ADR-0018 refuses to match a reference that takes the process down, and the rows pin that
//! the answer is the value's own `length`.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_ten_million_level_array_is_dropped_without_overflowing_the_stack_5782() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-nc",
            "reduce range(10000000) as $i ([]; .[0] = .) | length",
        ],
        None,
    )?;
    assert_eq!((stdout.as_str(), code), ("1\n", 0), "{stderr}");
    Ok(())
}
