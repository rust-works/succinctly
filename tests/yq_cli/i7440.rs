//! #7440: the `-e` rule for a negative `halt_error` code is jq's; `succinctly yq` keeps its own.
//!
//! Real yq has no `halt_error` (its lexer rejects it), so under `--jq-extensions` there is no
//! reference to follow and a negative code exits 0 with or without `-e`, as it did before #7440.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

#[test]
fn negative_halt_error_code_exits_zero_in_yq_mode_with_and_without_e_7440() -> Result<()> {
    for flags in [&["--jq-extensions"][..], &["--jq-extensions", "-e"][..]] {
        for (filter, want) in [("halt_error(-3)", 0), ("halt_error(3)", 3)] {
            let (out, err, code) = run_yq_stdin_with_stderr(filter, "a: 1\n", flags)?;
            assert_eq!(code, want, "{flags:?} {filter}: out {out:?} err {err:?}");
        }
    }
    Ok(())
}
