//! #6943: the rule that a literal after a stage that matched nothing is emitted once is yq's. jq's pipe
//! loops zero times over an empty stage, and `succinctly jq` does the same.

use anyhow::Result;

use crate::run_jq_stdin;

#[test]
fn jq_mode_does_not_re_emit_a_literal_over_an_empty_stage_6943() -> Result<()> {
    for filter in [
        ".a[] | 1",
        ".a[] | [1]",
        ".a[] | {\"k\":1}",
        "select(false) | 1",
        ".d[] | select(.x == 2) | \"n\"",
        "[.a[] | 1]",
    ] {
        let (stdout, code) = run_jq_stdin(filter, "{\"a\":[],\"d\":[{\"x\":1}]}", &["-c"])?;
        assert_eq!(code, 0, "{filter}");
        let want = if filter.starts_with('[') { "[]\n" } else { "" };
        assert_eq!(stdout, want, "{filter}");
    }
    Ok(())
}
