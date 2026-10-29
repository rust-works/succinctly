//! #4749's padding is yq's (`tests/yq_cli/i4749.rs`): jq's `del` takes the union of the paths and
//! never pads an array past the end, however many indices it is given. Captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_stdin;

#[test]
fn test_jq_del_of_several_indices_never_pads_4749() -> Result<()> {
    for (filter, want) in [
        ("del(.[5], .[0])", "[20,30]\n"),
        ("del(.[5], .[7])", "[10,20,30]\n"),
        ("del(.[5])", "[10,20,30]\n"),
        ("del(.[0], .[2])", "[20]\n"),
        // A negative index and a repeat are still the union: yq's replay is yq mode only.
        ("del(.[-1], .[5])", "[10,20]\n"),
        ("del(.[5], .[-1])", "[10,20]\n"),
        ("del(.[0], .[0])", "[20,30]\n"),
    ] {
        let (stdout, code) = run_jq_stdin(filter, "[10,20,30]", &["-c"])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter}");
    }
    Ok(())
}
