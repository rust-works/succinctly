//! #5629 folds a chain of comparisons from the right in yq mode only: jq 1.7.1 gives the six
//! operators no associativity and refuses `1 == 1 == true` (`unexpected ==`, exit 3), and so
//! does `succinctly jq`. Rows were captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_a_chain_of_comparisons_is_still_a_syntax_error_in_jq_mode_5629() -> Result<()> {
    for program in [
        "1 == 1 == true",
        "1 != 2 != 3",
        "1 < 2 < 3",
        "1 == 1 != 2",
        "1 + 1 == 2 == true",
    ] {
        let (out, _, code) = run_jq_full(&["-nc", program], None)?;
        assert_eq!((out.as_str(), code), ("", 3), "{program:?}");
    }
    let (out, _, code) = run_jq_full(&["-nc", "(1 == 1) == true"], None)?;
    assert_eq!((out.as_str(), code), ("true\n", 0));
    Ok(())
}
