//! #6699: jq reads `."a b"` as a string index, a space included (yq's lexer refuses it, and
//! `succinctly yq` follows, `tests/yq_cli/i6699.rs`). `succinctly jq` is unchanged.

use anyhow::Result;

use crate::run_jq_stdin;

#[test]
fn test_jq_reads_a_dotted_name_with_a_space_6699() -> Result<()> {
    for (filter, want) in [
        (r#"."a b""#, "1\n"),
        (r#".x."a b""#, "5\n"),
        (r#"."a b" = 9 | ."a b""#, "9\n"),
        (".\"\"", "4\n"),
    ] {
        let (stdout, code) =
            run_jq_stdin(filter, r#"{"a b": 1, "": 4, "x": {"a b": 5}}"#, &["-c"])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter}");
    }
    Ok(())
}
