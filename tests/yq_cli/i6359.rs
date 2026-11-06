//! #6359 in yq mode: the type selectors are jq-only builtins that `succinctly yq` accepts under
//! `--jq-extensions`; in path position they are static path targets on that route too, and the
//! answers are the ones the interpreted route gave.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

const DOC: &str = "a: null\nb: [1, ~, x, {c: null, d: true}]\ne: {f: 2.5, h: []}\n";

#[test]
fn test_type_selectors_in_path_position_under_jq_extensions_6359() -> Result<()> {
    for (filter, expected) in [
        ("[path(..|nulls)]", "[[\"a\"],[\"b\",1],[\"b\",3,\"c\"]]"),
        ("[path(..|scalars)] | length", "7"),
        (
            "del(..|nulls)",
            "{\"b\":[1,\"x\",{\"d\":true}],\"e\":{\"f\":2.5,\"h\":[]}}",
        ),
        (
            "(..|numbers) |= . + 1",
            "{\"a\":null,\"b\":[2,null,\"x\",{\"c\":null,\"d\":true}],\"e\":{\"f\":3.5,\"h\":[]}}",
        ),
    ] {
        let (stdout, stderr, code) =
            run_yq_stdin_with_stderr(filter, DOC, &["--jq-extensions", "-o", "json", "-I0"])?;
        assert_eq!(code, 0, "{filter}: {stderr}");
        assert_eq!(stdout.trim(), expected, "{filter}");
    }
    Ok(())
}
