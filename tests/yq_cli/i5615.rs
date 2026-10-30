//! #5615 changes how `succinctly jq` lexes a number before `..`; `succinctly yq` keeps stopping the
//! number there, and yq v4.53.3 refuses `1..e1` outright (`lexer: invalid input text "e1"`).

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn test_number_double_dot_field_stays_a_compile_error_in_yq_5615() -> Result<()> {
    let (out, code) = run_yq_stdin("1..e1", "a: 1\n", &[])?;
    assert_ne!(code, 0, "{out}");
    Ok(())
}
