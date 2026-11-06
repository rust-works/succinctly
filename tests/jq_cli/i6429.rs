//! #6429: a counted repeat over a body with a loop of its own is written copy by copy, and its text
//! doubles with each count of the upper bound, so an upper bound of 7 is past the size cap and keeps its text
//! (`docs/compliance/jq/limitations.md`). `{3,6}` is under it and agrees with jq 1.7.1
//! (`/usr/bin/jq -nc`); `{3,7}` answers as the pattern read as written does, which differs.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_a_counted_repeat_past_the_size_cap_keeps_its_text_6429() -> Result<()> {
    for (filter, expected) in [
        (
            r#""ba" | [match("((?:(b|)*|a)+){3,6}";"g") | .string]"#,
            r#"["b","",""]"#,
        ),
        (
            r#""ba" | [match("((?:(b|)*|a)+){3,7}";"g") | .string]"#,
            // jq 1.7.1 has ["b","",""]: the rewrite is given up, and the crate's reading differs.
            r#"["ba",""]"#,
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(
            (stdout.trim_end(), code),
            (expected, 0),
            "{filter}: stderr {stderr}"
        );
    }
    Ok(())
}
