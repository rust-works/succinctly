//! #5458: `array_to_map` is a yq operator and stays undefined in jq mode.

use anyhow::Result;

use crate::run_jq_stdin;

#[test]
fn test_array_to_map_stays_out_of_jq_mode_5458() -> Result<()> {
    let (stdout, code) = run_jq_stdin("array_to_map", "[1]", &["-c"])?;
    assert_eq!(stdout, "");
    assert_ne!(code, 0);
    Ok(())
}
