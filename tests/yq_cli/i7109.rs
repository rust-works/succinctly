//! #7109: the owned step for a slice plus a closed operand is jq mode's. `succinctly yq` keeps its
//! own route for the same UPDATE, and answers as yq v4.53.3 does (`yq '.[] as $i ireduce (.;
//! .[1:] + [$i])'` over `[1,2,3,4,5]` is `[1,2,3,4,5]`).

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn a_slice_plus_a_closed_operand_as_an_ireduce_update_is_yqs() -> Result<()> {
    let (stdout, code) = run_yq_stdin(
        ".[] as $i ireduce (.; .[1:] + [$i])",
        "[1,2,3,4,5]\n",
        &["-o=json", "-I0"],
    )?;
    assert_eq!(code, 0, "{stdout}");
    assert_eq!(stdout.trim_end(), "[1,2,3,4,5]");
    Ok(())
}
