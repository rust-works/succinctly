//! #5421: a pipe tail holding a `def` call is handed on in pieces in jq mode only. `succinctly yq`
//! hands it on whole, so the first-stage rule yq applies to a failing generator (it drops the
//! outputs the stage had already produced) reads the same stages as before. Real yq rejects `def`
//! and `try`, so there is no yq value to capture; the row pins the value `succinctly yq` gave
//! before the split existed, which is also the value `jq` 1.7.1 gives for the same program.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn test_def_holding_pipe_keeps_the_first_stage_rule_in_yq_mode_5421() -> Result<()> {
    let filter = "def f: . + 1; [([0,1,2,3] | .[]) | try (. * 10 | [., . + 1] | .[] | if . == 11 then error(1) else . end | . + 1 | . + 1 | . + 1 | . + 1 | . + 1 | f | . + 1) catch 0]";
    let (stdout, code) = run_yq_stdin(filter, "null\n", &["-o=json", "-I0", "--jq-extensions"])?;
    assert_eq!(
        (stdout.as_str(), code),
        ("[7,8,17,0,27,28,37,38]\n", 0),
        "`{filter}`"
    );
    Ok(())
}
