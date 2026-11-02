//! #5794: real yq has no `walk` (its lexer rejects it), so under `--jq-extensions` it means what
//! jq defines. Its object arm therefore reads one output per member in yq mode too, as in jq mode:
//! a mapping member's error after its first output is never reached.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

#[test]
fn walk_reads_one_output_per_mapping_member_in_yq_5794() -> Result<()> {
    let (out, err, code) = run_yq_stdin_with_stderr(
        "walk(if type == \"!!int\" then (., error(\"x\")) else . end)",
        "a: 1\nb:\n  c: 2\n",
        &["--jq-extensions", "-o", "json", "-I", "0"],
    )?;
    assert_eq!(
        (out.as_str(), code),
        ("{\"a\":1,\"b\":{\"c\":2}}\n", 0),
        "{err}"
    );
    Ok(())
}
