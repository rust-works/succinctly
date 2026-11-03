//! #6053 changed how jq mode reads `."key"`; yq mode still reads the text between the quotes raw,
//! escapes and `\(` included, so `.x."a\(1)"` selects the key `a\(1)` (a backslash, a `(`, a `1`
//! and a `)`). Expectation captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn a_quoted_step_with_an_interpolation_looking_key_is_read_raw_in_yq() -> Result<()> {
    let (out, code) = run_yq_stdin(
        r#".x."a\(1)""#,
        "x: {\"a\\\\(1)\": 9, \"a1\": 8}\n",
        &["-o=json", "-I=0"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(out.trim_end(), "9");
    Ok(())
}
