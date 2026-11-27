//! #6429: a counted repeat over a body with a loop of its own was written copy by copy on the
//! earlier engine, and its text doubled with each count of the upper bound, so an upper bound of 7
//! was past the size cap and kept its text, a recorded divergence: `{3,7}`, `{4,7}` and `{7}`
//! answered as the pattern read as written does. On Oniguruma every row matches jq 1.7.1 (#7965).
//! Every expectation was captured from jq 1.7.1 (`/usr/bin/jq -nc`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_a_counted_repeat_past_the_size_cap_answers_as_jq_does_6429() -> Result<()> {
    for (filter, expected) in [
        (
            r#""ba" | [match("((?:(b|)*|a)+){2,6}";"g") | .string]"#,
            r#"["b","",""]"#,
        ),
        (
            r#""ba" | [match("((?:(b|)*|a)+){6}";"g") | .string]"#,
            r#"["b","",""]"#,
        ),
        (
            r#""ba" | [match("((?:(b|)*|a)+){3,6}";"g") | .string]"#,
            r#"["b","",""]"#,
        ),
        (
            r#""ba" | [match("((?:(b|)*|a)+){3,7}";"g") | .string]"#,
            r#"["b","",""]"#,
        ),
        (
            r#""ba" | [match("((?:(b|)*|a)+){4,7}";"g") | .string]"#,
            r#"["b","",""]"#,
        ),
        (
            r#""ba" | [match("((?:(b|)*|a)+){7}";"g") | .string]"#,
            r#"["b","",""]"#,
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
