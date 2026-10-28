//! #5456: the yq spellings `~`, `toyaml`/`fromyaml` and `ireduce` do not exist in jq mode.

use anyhow::Result;

use crate::run_jq_stdin;

#[test]
fn test_yq_spellings_stay_out_of_jq_mode_5456() -> Result<()> {
    for filter in [
        "~",
        "1 as $x ireduce (0; .)",
        ".a | toyaml",
        ".a | fromyaml",
    ] {
        let (stdout, code) = run_jq_stdin(filter, r#"{"a":1}"#, &["-c"])?;
        assert_eq!(stdout, "", "`{filter}`");
        assert_ne!(code, 0, "`{filter}` must be rejected");
    }
    Ok(())
}
