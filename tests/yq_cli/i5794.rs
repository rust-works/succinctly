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

/// A mapping with a repeated key rewrites each live entry and leaves the earlier ones as read
/// (the object arm that keeps shadows), and still reads one output per member.
#[test]
fn walk_reads_one_output_per_member_of_a_mapping_with_a_repeated_key_in_yq_5794() -> Result<()> {
    let (out, err, code) = run_yq_stdin_with_stderr(
        "walk(if type == \"!!int\" then (., error(\"x\")) else . end)",
        "a: 1\na: 2\nb: 3\n",
        &["--jq-extensions"],
    )?;
    assert_eq!((out.as_str(), code), ("a: 1\na: 2\nb: 3\n", 0), "{err}");
    Ok(())
}

/// With nothing produced before the error, a member still raises, in yq mode as in jq mode.
#[test]
fn walk_raises_an_error_a_member_produces_before_any_value_in_yq_5794() -> Result<()> {
    let (out, err, code) = run_yq_stdin_with_stderr(
        "walk(if type == \"!!int\" then (error(\"x\"), .) else . end)",
        "a: 1\n",
        &["--jq-extensions"],
    )?;
    assert_eq!(
        (out.as_str(), code, err.contains('x')),
        ("", 1, true),
        "{err}"
    );
    Ok(())
}
