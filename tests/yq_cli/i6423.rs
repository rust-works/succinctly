//! #6423: jq mode words a regex compile error as jq does (`Regex failure: ...`); yq mode keeps Go's
//! words, as yq does. Every row was captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

#[test]
fn a_refused_pattern_keeps_go_s_wording_6423() -> Result<()> {
    for (pattern, message) in [
        ("[", "missing closing ]: `[`"),
        ("(", "missing closing ): `(`"),
        ("a{2,1}", "invalid repeat count: `{2,1}`"),
    ] {
        let filter = format!(".a | test({})", serde_json::to_string(pattern)?);
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(&filter, "a: x\n", &[])?;
        assert_eq!((stdout.as_str(), code), ("", 1), "{pattern}");
        assert_eq!(
            stderr.trim_end(),
            format!("Error: error parsing regexp: {message}"),
            "{pattern}"
        );
    }
    Ok(())
}
