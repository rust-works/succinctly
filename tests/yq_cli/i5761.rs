//! #5761: a jq-mode container operand of `op=` / `//=` keeps its storage; `succinctly yq` is
//! unchanged and still rebuilds it, so these answers are the ones it gave before.

use anyhow::Result;

use crate::run_yq_stdin;

const DOC: &str = "a: [1]\nm: {x: 1}\n";

#[test]
fn test_yq_compound_store_of_a_container_operand_is_unchanged_5761() -> Result<()> {
    for (filter, expected) in [
        (".a += [2]", "a: [1, 2]\nm: {x: 1}\n"),
        (
            r#".b //= [3, {"x": null}]"#,
            "a: [1]\nm: {x: 1}\nb:\n  - 3\n  - x: null\n",
        ),
        (".a *=+ [9]", "a: [1, 9]\nm: {x: 1}\n"),
        (r#".m += {"y": [1]}"#, "a: [1]\nm: {x: 1, y: [1]}\n"),
    ] {
        // `//=` is not yq's operator, so it needs the flag (#6981).
        let args: &[&str] = if filter.contains("//=") {
            &["--jq-extensions"]
        } else {
            &[]
        };
        let (stdout, code) = run_yq_stdin(filter, DOC, args)?;
        assert_eq!((stdout.as_str(), code), (expected, 0), "{filter}");
    }
    Ok(())
}
