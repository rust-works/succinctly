//! #6684: `_nwise` is jq's builtin (its builtin.jq defines it), and `succinctly jq` has it now.
//! Real yq has no such function, so `succinctly yq` still refuses it, with or without
//! `--jq-extensions`: the expansion is jq mode's.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn test_yq_refuses_nwise_with_and_without_jq_extensions_6684() -> Result<()> {
    for extra in [&[][..], &["--jq-extensions"][..]] {
        for filter in ["_nwise(2)", "_nwise(.; 2)"] {
            let (stdout, code) = run_yq_stdin(filter, "[1, 2, 3]\n", extra)?;
            assert_eq!(stdout, "", "{filter} {extra:?}");
            assert_ne!(code, 0, "{filter} {extra:?}");
        }
    }
    Ok(())
}
