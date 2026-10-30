//! #5553: `succinctly jq -C` colors the line `debug` writes to stderr, as jq does. `succinctly yq`
//! keeps writing it plain: the renderer is installed by the jq subcommand only.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

#[test]
fn test_yq_debug_line_stays_uncolored_under_dash_c_5553() -> Result<()> {
    let (_, stderr, code) =
        run_yq_stdin_with_stderr(".a | debug", "a: 1\n", &["--jq-extensions", "-C"])?;
    assert_eq!(code, 0);
    assert_eq!(stderr, "[\"DEBUG:\",1]\n");
    Ok(())
}
