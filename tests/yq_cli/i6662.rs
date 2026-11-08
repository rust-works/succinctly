//! #6662: `isvalid(f)` is a succinctly extension, and `succinctly yq` gates it behind
//! `--jq-extensions`, as it does every jq-only name real yq's lexer rejects. yq v4.53.3 answers
//! `Error: 1:1: lexer: invalid input text "isvalid(.a)"` (exit 1); succinctly refuses the name with
//! the same exit code, and runs it only with the flag.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

#[test]
fn isvalid_is_refused_in_yq_mode_without_the_extensions_flag_6662() -> Result<()> {
    let (out, err, code) = run_yq_stdin_with_stderr("isvalid(.a)", "a: 1\n", &[])?;
    assert_eq!(out, "");
    assert_eq!(code, 1);
    assert!(
        err.contains("\"isvalid\" is not part of yq's syntax") && err.contains("--jq-extensions"),
        "{err}"
    );
    Ok(())
}

#[test]
fn isvalid_runs_in_yq_mode_with_the_extensions_flag_6662() -> Result<()> {
    for (filter, want) in [
        ("isvalid(.a)", "true\n"),
        ("isvalid(.a | error)", "false\n"),
    ] {
        let (out, err, code) = run_yq_stdin_with_stderr(filter, "a: 1\n", &["--jq-extensions"])?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (want, "", 0),
            "{filter}"
        );
    }
    Ok(())
}
