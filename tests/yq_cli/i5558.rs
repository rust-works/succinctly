//! #5558: `succinctly jq -S`/`-a` apply to the line `debug` writes to stderr, as jq does.
//! `succinctly yq` writes it as it did: the renderer is installed by the jq subcommand only.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

#[test]
fn test_yq_debug_line_is_unchanged_under_sort_keys_and_ascii_5558() -> Result<()> {
    for flag in ["-S", "-a"] {
        let (_, stderr, code) = run_yq_stdin_with_stderr(
            ".b | debug | empty",
            "b: {z: 1, a: é}\n",
            &["--jq-extensions", flag],
        )?;
        assert_eq!(code, 0, "{flag}");
        assert_eq!(stderr, "[\"DEBUG:\",{\"z\":1,\"a\":\"é\"}]\n", "{flag}");
    }
    Ok(())
}
